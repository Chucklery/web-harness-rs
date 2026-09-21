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

## workspace_info

Returns the canonical configured workspace root.

## read_files

Reads up to 16 UTF-8 files per call.

Each path may be a string for legacy whole-file reads or an object with `start_line`, `end_line`, and `expected_read_revision`. Results include a bounded `read_revision`; truncated results provide parser-ready continuation parameters. A continuation whose revision no longer matches is rejected as a conflict. Common protected paths (`.env`, private keys, credentials, and similar files) return an approval ticket; retry with its `approval_id` after host confirmation.

Current limits:

- 256 KiB per file
- 512 KiB total per batch
- all paths must remain inside the workspace

## list_files

Lists sorted workspace-relative files, directories, and symlinks without executing a shell command. It defaults to Git-tracked paths and supports bounded pagination, an `all` source, and a simple include glob. If enumeration itself hits its hard scan bound, the result is marked truncated and does not provide a continuation offset.

## search

Uses the system ripgrep binary and returns at most 200 bounded matches.

ripgrep is an external runtime dependency, not a bundled one. When `rg` is missing from `PATH` the tool returns a dependency error naming ripgrep and how to install it; all other tools continue to work. Homebrew installations declare ripgrep as a formula dependency.

The existing `search` tool supports either one `query` or a `queries` batch of 1 to 8 strings. The two forms are mutually exclusive. Batch queries share one `max_results` budget for the entire response rather than multiplying the limit per query. This reduces ChatGPT Web ↔ local MCP round trips while keeping response size and tunnel traffic bounded.

Search also accepts a workspace-relative `scope`, literal mode, bounded include/exclude globs, and `matches`, `files_with_matches`, or `count` output modes. It continues to invoke the system ripgrep process per request and does not maintain an index.

Single-query searches accept an `offset` continuation and return `next_offset`; keep the query, scope, glob filters, mode, and result limit unchanged when continuing. If ripgrep output itself reaches its hard bound, the result is marked truncated without a continuation.

An explicit search scope that is itself protected requires the same one-time approval as a protected file read; ordinary workspace-wide searches continue to filter protected paths.

## workspace_instructions

Discovers AGENTS.md files from the workspace root down to the target path and returns them in root-to-leaf order.

## patch

Applies bounded Codex-style Add File, Update File, and Delete File operations. Paths are workspace scoped, Add File creates missing parent directories only after boundary checks, updates require matching context, ambiguous hunks are rejected, and writes use same-directory temporary files followed by rename. `expected_read_revisions` optionally fences Update File operations against a prior read.

## exec

Executes argv-based commands inside the workspace. It also provides an explicit `script` mode using the platform shell (`sh`/`bash` on Unix, `powershell`/`pwsh` on Windows); the bounded script is passed as one-shot stdin and always requires approval. Shell-string mode remains absent for argv calls: the command is validated before it starts, and an inline-evaluation flag (`sh -c`, `bash -c`, `python -c`, `ruby -c`) on a known shell or interpreter is rejected. This is a policy filter rather than a security boundary — `PATH` can still contain a wrapper such as `env` — so the sandbox below remains the actual boundary.

Direct Git commands sent through exec are classified before spawn and use the same approval capabilities as structured Git: `git push` requires `git.remote.write`; every other direct Git invocation conservatively requires `git.local.write`. An approved executable or wrapper may still mutate files inside the sandboxed workspace, so approval of general process execution grants that workspace-scoped authority.

On macOS, native Seatbelt is used when available. On systems without a native sandbox backend, execution requires an explicit one-time approval.

Network policy is per execution and defaults to `deny`. `outbound` adds only Seatbelt's outbound-network permission and always requires a one-time approval bound to the exact argv, cwd, background mode, capability, and network policy. The upgrade is rejected when no native sandbox backend can enforce it; there is no unsandboxed network mode.

The macOS profile allows writes only inside the workspace, TMPDIR, `/tmp`, `/private/tmp`, and `/dev/null`. `/dev/null` is granted explicitly because shells, Git, and most compiler and build toolchains open it unconditionally; without it `git status` and similar commands fail with `Operation not permitted`.

Child processes receive a `WEB_HARNESS_SANDBOX` marker. If web-harness execution itself runs inside a web-harness sandbox, the marker prevents a second Seatbelt profile from being applied — macOS rejects nested `sandbox-exec` with `sandbox_apply: Operation not permitted`, which previously broke `cargo test` run through `exec`. The marker suppresses re-wrapping only; the outer sandbox remains enforced.

Foreground commands have a maximum 10 minute timeout and bounded stdout/stderr results. Background execution is limited to two concurrent jobs.

Exec optionally accepts one-shot UTF-8 `stdin` input up to 64 KiB. The input is staged through a host-managed temporary file, then closed; PTY and interactive sessions are not provided. The exact input is bound into approval tickets.

## job

Polls, waits for, lists, cancels, or reads bounded stdout/stderr tails from background jobs. The output action accepts a byte cursor for incremental reads and returns the next cursor. Job output spills to temporary files instead of growing without bound in memory. Owned process groups are terminated when the host exits.

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

Mutation calls may provide `expected_head`; after approval and immediately before execution, the current HEAD must still match or the operation returns a conflict.

`diff` accepts optional `offset` and `limit` arguments to return bounded file/hunk chunks with a parser-ready `next_offset`; keep the other diff arguments unchanged when continuing. `show_file` reads one workspace-scoped path from a validated revision. Commit accepts an optional pathspec list so existing staged changes outside that list are not included. Commit messages, refs, remotes, refspecs, and pathspec counts are bounded. Arbitrary Git argv is not exposed.

## permission

Approves or denies one-time request-bound tickets. Tickets expire after five minutes and are consumed after one successful authorization.

The tool is declared as destructive through MCP `ToolAnnotations`, so ChatGPT asks the user to confirm the approval call before it reaches web-harness. The annotation is the user-interaction boundary; the in-process permission engine remains the server-side binding and single-use boundary.

`permission` is direct-only. The adaptive `call_runtime_tool` gateway rejects it so the gateway's generic annotation cannot bypass host confirmation.

The same ticket cannot be reused for a different command or Git operation.

A ticket that fails to match the request is *not* consumed. An approval is consumed only when it is successfully used, or when it expires. A mismatching request returns an error and leaves the still-valid approval in place, so a client that mis-specifies a command can retry with the correct payload without re-approving. Denial removes the ticket explicitly.
