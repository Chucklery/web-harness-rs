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
- structured read-only Git operations
- one-time approval-bound execution

Native OS sandbox enforcement and real Secure MCP Tunnel end-to-end acceptance remain roadmap work.

## Design target

The target is a local runtime that remains understandable, bounded, and suitable for low-memory Macs. Performance claims are not considered valid until measured on real 8 GB Intel and Apple Silicon machines.

