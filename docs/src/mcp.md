# MCP Tool Model

The project deliberately keeps a small number of high-value tools rather than exposing a large command surface.

## Adaptive Runtime compatibility controls

For ChatGPT connector clients that expect a WebCodex-style control plane, the server also exposes:

- `runtime_status`
- `work_on_project`
- `tool_manifest`
- `call_runtime_tool`

These are compatibility controls, not a second execution system. `call_runtime_tool` uses a fixed allowlist and forwards only to the normal bounded runtime tools below. `work_on_project` cannot switch the server to an arbitrary path; it accepts only the workspace configured when `web-harness serve --stdio` starts.

Tool results provide bounded `structuredContent` alongside the text content compatibility projection. Clients should prefer the structured field and treat text as display/fallback data.

The stdio protocol accepts one UTF-8 JSON line at a time and caps each incoming line at 2 MiB. Oversized lines are drained and rejected so a subsequent request can still be processed.

`tools/list` includes MCP `ToolAnnotations` as conservative interaction hints. They describe read-only, destructive, and open-world behavior for Host UX; they are not the security boundary. Capability checks, Host elicitation, workspace guards, and the OS sandbox remain authoritative.

## workspace_info

Returns the canonical configured workspace root.

## read_files

Reads up to 16 UTF-8 files per call.

Each path may be a string for legacy whole-file reads or an object with `start_line`, `end_line`, and `expected_read_revision`. Results include a bounded metadata-plus-content-sample `read_revision`; truncated results provide parser-ready continuation parameters. A continuation whose revision no longer matches is rejected as a conflict. Batch items return independently: a missing, denied, non-regular, non-UTF-8, out-of-range, or otherwise unreadable path carries its own structured `error` while other items still return their content. The 512 KiB total content limit remains shared across successful items; later paths receive `limit_exceeded` once no batch budget remains. Common protected paths (`.env`, private keys, credentials, and similar files) require an approval ticket; retry with its `approval_id` after host confirmation.

Current limits:

- 256 KiB per file
- 512 KiB total per batch
- all paths must remain inside the workspace

A complete line must fit within the remaining page and batch byte budget. If a single line does not fit, that item returns `limit_exceeded` instead of a continuation that cannot advance.

## list_files

Lists sorted workspace-relative files, directories, and symlinks without executing a shell command. It defaults to Git-tracked paths and supports bounded pagination, an `all` source, and a simple include glob. If enumeration itself hits its hard scan bound, the result is marked truncated and does not provide a continuation offset.

## search

Uses the system ripgrep binary and returns at most 200 bounded matches.

ripgrep is an external runtime dependency, not a bundled one. When `rg` is missing from `PATH` the tool returns a dependency error naming ripgrep and how to install it; all other tools continue to work. Homebrew installations declare ripgrep as a formula dependency.

The existing `search` tool supports either one `query` or a `queries` batch of 1 to 8 strings. The two forms are mutually exclusive. Batch queries share one `max_results` budget for the entire response rather than multiplying the limit per query. A failure for one batch query is returned beside that query and does not prevent the remaining queries from running; entries skipped after the shared result budget is exhausted are marked `result_budget_exhausted`. This reduces ChatGPT Web ↔ local MCP round trips while keeping response size and tunnel traffic bounded.

Search also accepts a workspace-relative `scope`, literal mode, bounded include/exclude globs, and `matches`, `files_with_matches`, or `count` output modes. In `matches` mode, optional `context_lines` returns up to two nearby lines on either side of each hit; nearby lines are redacted and share a 128 KiB serialized-output budget. If that budget or ripgrep's output cap is reached, affected hits report `context_truncated`. It continues to invoke the system ripgrep process per request and does not maintain an index.

Single-query searches accept an `offset` continuation and return `next_offset`; keep the query, scope, glob filters, mode, and result limit unchanged when continuing. If ripgrep output itself reaches its hard bound, the result is marked truncated without a continuation.

An explicit search scope that is itself protected requires the same one-time approval as a protected file read; ordinary workspace-wide searches continue to filter protected paths.

## workspace_instructions

Discovers AGENTS.md files from the workspace root down to the target path and returns them in root-to-leaf order.

## patch

Applies bounded Codex-style Add File, Update File, and Delete File operations. Paths are workspace scoped, Add File creates missing parent directories only after boundary checks, updates require matching context, ambiguous hunks are rejected, and writes use same-directory temporary files followed by rename. A patch contains at most 256 operations and returns changed paths with per-file added/removed byte counts. `expected_read_revisions` optionally fences Update File operations against a prior read and is rechecked after reading and staging, immediately before commit.

## exec

Executes argv-based commands inside the workspace. It also provides an explicit `script` mode using the platform shell (`sh`/`bash` on Unix, `powershell`/`pwsh` on Windows); the bounded script is passed as one-shot stdin and always requires approval. Shell-string mode remains absent for argv calls: the command is validated before it starts, and an inline-evaluation flag (`sh -c`, `bash -c`, `python -c`, `ruby -c`) on a known shell or interpreter is rejected. This is a policy filter rather than a security boundary — `PATH` can still contain a wrapper such as `env` — so the sandbox below remains the actual boundary.

Direct Git commands sent through exec are classified before spawn and use the same approval capabilities as structured Git: `git push` requires `git.remote.write`; every other direct Git invocation conservatively requires `git.local.write`. Script mode conservatively detects literal Git mutation tokens and requires a separate `git_approval_id` for `git.local.write` or `git.remote.write`, in addition to the script and (if requested) outbound-network approvals. This text scan is only a policy prompt, not a security boundary; the Host-approved script and OS sandbox remain authoritative. An approved executable or wrapper may still mutate files inside the sandboxed workspace, so approval of general process execution grants that workspace-scoped authority.

On macOS, native Seatbelt is used when available. On systems without a native sandbox backend, execution requires an explicit one-time approval.

Network policy is per execution and defaults to `deny`. `outbound` adds only Seatbelt's outbound-network permission and always requires a separate one-time `network.outbound` approval bound to the exact argv, cwd, background mode, and network policy. If the same command also needs process or Git approval, each capability has its own ticket and Host confirmation; the network ticket is passed as `network_approval_id` and cannot authorize a Git mutation, while the Git ticket cannot authorize network access. A Git mutation found in script mode therefore needs its own `git_approval_id`; outbound approval alone does not satisfy `git.local.write` or `git.remote.write`. The upgrade is rejected when no native sandbox backend can enforce it; there is no unsandboxed network mode.

The macOS profile allows writes only inside the workspace, TMPDIR, `/tmp`, `/private/tmp`, and `/dev/null`. `/dev/null` is granted explicitly because shells, Git, and most compiler and build toolchains open it unconditionally; without it `git status` and similar commands fail with `Operation not permitted`.

Child processes receive a `WEB_HARNESS_SANDBOX` marker. If web-harness execution itself runs inside a web-harness sandbox, the marker prevents a second Seatbelt profile from being applied — macOS rejects nested `sandbox-exec` with `sandbox_apply: Operation not permitted`, which previously broke `cargo test` run through `exec`. The marker suppresses re-wrapping only; the outer sandbox remains enforced.

Foreground commands have a maximum 10 minute timeout and bounded stdout/stderr results. Background execution is limited to two concurrent jobs.

Exec optionally accepts one-shot UTF-8 `stdin` input up to 64 KiB. The input is staged through a host-managed temporary file, then closed; child stdout and stderr are drained through bounded in-memory buffers, so noisy jobs cannot grow unbounded output state. PTY and interactive sessions are not provided. The exact input is bound into approval tickets.

## job

Polls, waits for, lists, cancels, or reads bounded stdout/stderr tails from background jobs. `wait` accepts separate `stdout_cursor` and `stderr_cursor` values and returns the status plus new output from both streams in one call. The `output` action accepts a byte cursor for a single stream and returns the next cursor. The host drains child pipes continuously but retains each stream in a bounded in-memory ring buffer; a cursor older than the retained window is reported as truncated. Owned process groups are terminated when the host exits.

## git

Provides structured actions only.

Read-only actions:

- status
- diff
- log
- show
- show_file

Local mutation actions:

- add
- commit
- switch
- create_branch
- restore

Remote mutation:

- push

Every mutation requires a one-time approval bound to the exact generated Git argv. Push receives a stronger approval description because it changes a remote repository.

Mutation calls may provide `expected_head`; commit requires it. When supplied, the current HEAD is checked before approval is requested and again immediately before execution, otherwise the operation returns a conflict.

`head` returns the full current commit ID for optimistic-concurrency fences. `diff` accepts optional `offset` and `limit` arguments to return bounded file/hunk chunks with a parser-ready `next_offset`; keep the other diff arguments unchanged when continuing. Before returning a diff, the host scans changed path names with a separate bounded Git query; if a changed path is protected, the complete protected path set is bound to a one-time Host approval and no diff content is returned before approval. `show_file` reads one workspace-scoped path from a validated revision in bounded line pages (`start_line`, up to 256 KiB), and returns `next_start_line` when another page is available; protected paths require the same approval. Each call captures at most 8 MiB from the Git blob; a source larger than that is marked `source_truncated` without an unsafe continuation. Commit accepts an optional pathspec list so existing staged changes outside that list are not included, and requires `expected_head` to fence approval against concurrent branch changes. Commit messages, refs, remotes, refspecs, and pathspec counts are bounded. Arbitrary Git argv is not exposed.

## Errors

Recoverable tool failures, including invalid arguments, missing files, denied paths, non-regular files, out-of-range reads, encoding failures, workspace conflicts, permission denials, dependency failures, and command failures, return a normal `tools/call` result with `isError: true` and machine-readable `structuredContent.error`. JSON-RPC errors remain reserved for malformed protocol requests, unknown methods, and requests that cannot be dispatched to a tool.

## Approval

One-time approvals are host-only. When the initialized MCP client advertises elicitation support, web-harness sends `elicitation/create` with a bounded confirmation form and only executes after the client returns an accepted response. The approval ticket is never exposed as a callable MCP tool, so the model cannot approve its own request or route approval through `call_runtime_tool`.

Clients without elicitation support receive `approval_required` and must wait for a host-side approval mechanism; web-harness does not silently execute the operation. When that Host has no confirmation flow, the user must open a separate local terminal outside the MCP `exec` tool and run `web-harness approvals`, then `web-harness approve <ticket-id>`. The CLI displays the capability and summary and requires an interactive `y` confirmation. Running it through MCP `exec` is not an approval path: on macOS the state directory is intentionally outside the child sandbox's writable paths, and elsewhere tool execution still requires its own approval. The ticket and its request digest remain in MCP-process memory; a bounded, private state directory carries only the short-lived summary and one-shot CLI approval marker. The CLI refuses non-interactive stdin/stdout and never appears as an MCP tool. Tickets expire after five minutes and are consumed after one successful authorization.

During an approval prompt, an elicitation `decline` returns a denied result; an elicitation `cancel` or `notifications/cancelled` for the active `tools/call` abandons that call without a response and revokes its pending and previously collected tickets. Cancellation is currently observed while waiting for Host approval; synchronous tool execution already in progress is not cooperatively interrupted and remains bounded by the tool's own timeout. This behavior still needs verification with a real ChatGPT client.

The same ticket cannot be reused for a different command or Git operation. A ticket that fails to match the request is *not* consumed. Denial removes the ticket explicitly.
