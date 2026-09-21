# MCP Tool Model

The project deliberately keeps a small number of high-value tools rather than exposing a large command surface.

## Adaptive Runtime compatibility controls

For ChatGPT connector clients that expect a WebCodex-style control plane, the server also exposes:

- `runtime_status`
- `work_on_project`
- `tool_manifest`
- `call_runtime_tool`

These are compatibility controls, not a second execution system. `call_runtime_tool` uses a fixed allowlist and forwards only to the normal bounded runtime tools below. `work_on_project` cannot switch the server to an arbitrary path; it accepts only the workspace configured when `web-harness serve --stdio` starts.

## workspace_info

Returns the canonical configured workspace root.

## read_files

Reads up to 16 UTF-8 files per call.

Current limits:

- 256 KiB per file
- 512 KiB total per batch
- all paths must remain inside the workspace

## search

Uses the system ripgrep binary and returns at most 200 bounded matches.

ripgrep is an external runtime dependency, not a bundled one. When `rg` is missing from `PATH` the tool returns a dependency error naming ripgrep and how to install it; all other tools continue to work. Homebrew installations declare ripgrep as a formula dependency.

The existing `search` tool supports either one `query` or a `queries` batch of 1 to 8 strings. The two forms are mutually exclusive. Batch queries share one `max_results` budget for the entire response rather than multiplying the limit per query. This reduces ChatGPT Web ↔ local MCP round trips while keeping response size and tunnel traffic bounded.

## workspace_instructions

Discovers AGENTS.md files from the workspace root down to the target path and returns them in root-to-leaf order.

## patch

Applies bounded Codex-style Add File, Update File, and Delete File operations. Paths are workspace scoped, updates require matching context, ambiguous hunks are rejected, and writes use same-directory temporary files followed by rename.

## exec

Executes argv-based commands inside the workspace. Shell-string mode is intentionally absent: the command is validated before it starts, and an inline-evaluation flag (`sh -c`, `bash -c`, `python -c`, `ruby -c`) on a known shell or interpreter is rejected with a message that points at the script-file form in the workspace. This is a policy filter rather than a security boundary — `PATH` can still contain a wrapper such as `env` — so the sandbox below remains the actual boundary.

Direct Git commands sent through exec are classified before spawn and use the same approval capabilities as structured Git: `git push` requires `git.remote.write`; every other direct Git invocation conservatively requires `git.local.write`. An approved executable or wrapper may still mutate files inside the sandboxed workspace, so approval of general process execution grants that workspace-scoped authority.

On macOS, native Seatbelt is used when available. On systems without a native sandbox backend, execution requires an explicit one-time approval.

The macOS profile allows writes only inside the workspace, TMPDIR, `/tmp`, `/private/tmp`, and `/dev/null`. `/dev/null` is granted explicitly because shells, Git, and most compiler and build toolchains open it unconditionally; without it `git status` and similar commands fail with `Operation not permitted`.

Child processes receive a `WEB_HARNESS_SANDBOX` marker. If web-harness execution itself runs inside a web-harness sandbox, the marker prevents a second Seatbelt profile from being applied — macOS rejects nested `sandbox-exec` with `sandbox_apply: Operation not permitted`, which previously broke `cargo test` run through `exec`. The marker suppresses re-wrapping only; the outer sandbox remains enforced.

Foreground commands have a maximum 10 minute timeout and bounded stdout/stderr results. Background execution is limited to two concurrent jobs.

## job

Polls, cancels, or reads bounded stdout/stderr tails from background jobs. Job output spills to temporary files instead of growing without bound in memory. Owned process groups are terminated when the host exits.

## git

Provides structured actions only.

Read-only actions:

- status
- diff
- log
- show

Local mutation actions:

- add
- commit
- switch
- restore

Remote mutation:

- push

Every mutation requires a one-time approval bound to the exact generated Git argv. Push receives a stronger approval description because it changes a remote repository.

Commit messages, refs, remotes, refspecs, and pathspec counts are bounded. Arbitrary Git argv is not exposed.

## permission

Approves or denies one-time request-bound tickets. Tickets expire after five minutes and are consumed after one successful authorization.

The tool is declared as destructive through MCP `ToolAnnotations`, so ChatGPT asks the user to confirm the approval call before it reaches web-harness. The annotation is the user-interaction boundary; the in-process permission engine remains the server-side binding and single-use boundary.

`permission` is direct-only. The adaptive `call_runtime_tool` gateway rejects it so the gateway's generic annotation cannot bypass host confirmation.

The same ticket cannot be reused for a different command or Git operation.

A ticket that fails to match the request is *not* consumed. An approval is consumed only when it is successfully used, or when it expires. A mismatching request returns an error and leaves the still-valid approval in place, so a client that mis-specifies a command can retry with the correct payload without re-approving. Denial removes the ticket explicitly.
