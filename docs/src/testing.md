# Testing

Current automated tests cover:

- normal in-workspace path resolution
- symlink escape rejection on Unix
- bounded reads
- scoped AGENTS discovery
- structured patch add/update/delete and conflicts
- foreground and background process execution
- Git status gateway
- approval ticket binding and one-time consumption
- an end-to-end stdio MCP initialization and tool-list test
- an approved background-job run followed by MCP disconnect and process-group cleanup verification
- a tunnel doctor that validates local MCP roundtrip before external tunnel acceptance
- a bounded external tunnel-evidence contract; a successful wrapper exit without complete evidence is not accepted

The project should grow toward five layers:

1. unit tests
2. property tests
3. integration tests
4. tunnel end-to-end tests
5. real-machine resource benchmarks

Resource claims for 8 GB Macs require physical-machine evidence rather than CI assumptions.

The built-in benchmark command emits machine-readable JSON and measures local operation latency plus MCP-ready startup/RSS/CPU. It still cannot evaluate Tunnel + Host RSS by itself, and a complete 8 GB gate requires physical Intel and Apple Silicon evidence. See [Performance and 8 GB Gates](benchmarks.md).
