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

Status: implemented for argv-based execution; shell-string mode is intentionally omitted.

## Stage 5

- background jobs
- bounded in-memory output
- disk spill artifacts

Status: implemented for host-owned jobs with bounded returned tails and temporary-file spill.

## Stage 6

- structured Git gateway

Status: implemented as read-only status/diff/log/show.

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

Status: the tunnel acceptance harness and benchmark harness exist. Secret redaction and child environment minimization are implemented. One physical 8 GiB Intel Mac release-mode snapshot is checked in; Apple Silicon 8 GiB and Tunnel + Host RSS evidence remain pending.

UI and LSP remain optional and should only be added if real usage metrics justify them.

