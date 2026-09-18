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
- a tunnel doctor that validates local MCP roundtrip before external tunnel acceptance

The project should grow toward five layers:

1. unit tests
2. property tests
3. integration tests
4. tunnel end-to-end tests
5. real-machine resource benchmarks

Resource claims for 8 GB Macs require physical-machine evidence rather than CI assumptions.

The built-in benchmark command is only a local kernel microbenchmark. It is not evidence for tunnel latency or the 8 GB memory gates.

