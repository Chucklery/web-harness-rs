# Roadmap

## Stage 1

- Rust workspace
- MCP stdio
- path guard
- bounded reads

Status: implemented.

## Stage 2

- ripgrep search
- scoped AGENTS discovery

Status: implemented.

## Stage 3

- Codex-style structured patch parser
- safe apply
- compact diff result

Status: structured add/update/delete patching is implemented. Compact Git diff remains a separate Git tool call by design.

## Stage 4

- bounded process execution
- timeout and cancellation
- process group handling

Status: implemented for argv-based execution and an explicitly approved, bounded one-shot script mode; interactive shell-string mode and PTY sessions are intentionally omitted.

## Stage 5

- background jobs
- bounded in-memory output
- disk spill artifacts

Status: implemented for host-owned jobs with bounded returned tails, temporary-file spill, output cursors, bounded waits, and optional one-shot stdin.

## Stage 6

- structured Git gateway

Status: implemented with read-only status/diff/log/show/show_file operations, bounded diff pagination, branch creation, and approval-gated add/commit/switch/restore/push mutations.

## Stage 7

- shared permission engine
- approval tickets
- macOS sandbox

Status: approval-bound execution is implemented and structured workspace patches pass through the permission kernel. On macOS, /usr/bin/sandbox-exec is used for deny-by-default Seatbelt enforcement with workspace-scoped writes; systems without a native backend fall back to explicit approval.

## Stage 8 and later

- real ChatGPT Secure MCP Tunnel end-to-end acceptance
- physical-machine 8 GB resource benchmarks
- sandbox hardening
- performance hardening

Status: this stage is not complete. Release bundles include the pinned official OpenAI tunnel-client runtime, and local tunnel acceptance/benchmark harnesses exist, but real ChatGPT Secure MCP Tunnel end-to-end acceptance is not yet verified. Secret redaction and child environment minimization are implemented. One physical 8 GiB Intel Mac release-mode snapshot is checked in; Apple Silicon 8 GiB and Tunnel + Host RSS evidence remain pending. Native sandboxing is implemented only for macOS Seatbelt; Linux Landlock evaluation and native Linux/Windows sandbox implementations are not complete. Broader performance hardening is not complete.

UI and LSP are not implemented and remain optional; add them only if real usage metrics justify them.
