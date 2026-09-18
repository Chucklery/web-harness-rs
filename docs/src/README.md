# web-harness

web-harness is a lightweight local execution host for connecting ChatGPT Web to a real local code workspace through MCP.

The project keeps responsibilities deliberately narrow:

- ChatGPT handles reasoning, planning, and tool orchestration.
- web-harness handles local workspace access and execution primitives.
- OpenAI Secure MCP Tunnel is the intended remote transport.
- stdio is the local MCP transport.

## Current implementation

The current pre-1.0 implementation includes:

- a Rust CLI
- workspace validation
- canonical path guarding
- MCP stdio JSON-RPC host
- workspace_info
- bounded read_files
- ripgrep-backed search
- scoped AGENTS discovery
- structured patching
- bounded foreground/background execution
- background job lifecycle and output tails
- structured read-only and approval-gated Git mutation operations
- one-time approval-bound execution and Git mutations
- bundled official OpenAI tunnel-client runtime in release/Homebrew distributions

macOS Seatbelt enforcement is implemented. Real Secure MCP Tunnel production acceptance and complete 8 GiB Intel/Apple Silicon release evidence remain release gates.

## Design target

The target is a local runtime that remains understandable, bounded, and suitable for low-memory Macs. Performance claims are not considered valid until measured on real 8 GB Intel and Apple Silicon machines.

