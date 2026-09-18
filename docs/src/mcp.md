# MCP Tool Model

The project deliberately keeps a small number of high-value tools rather than exposing a large command surface.

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

## workspace_instructions

Discovers AGENTS.md files from the workspace root down to the target path and returns them in root-to-leaf order.

## patch

Applies bounded Codex-style Add File, Update File, and Delete File operations. Paths are workspace scoped, updates require matching context, ambiguous hunks are rejected, and writes use same-directory temporary files followed by rename.

## exec

Executes argv-based commands inside the workspace. Shell-string mode is intentionally absent.

On macOS, native Seatbelt is used when available. On systems without a native sandbox backend, execution requires an explicit one-time approval.

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

The same ticket cannot be reused for a different command or Git operation.
