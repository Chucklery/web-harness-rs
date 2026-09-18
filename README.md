# web-harness

web-harness is a lightweight local Codex-style execution host designed to let ChatGPT Web work with a real local repository through MCP while keeping the local runtime small.

The project is intentionally not a second agent runtime. ChatGPT remains the agent; web-harness provides local execution capabilities.

Status: active pre-1.0 development. The current implementation provides a Rust CLI, MCP stdio host, workspace/path guards, scoped AGENTS discovery, bounded reads/search, structured patching, bounded foreground/background execution, a JobManager, a read-only structured Git gateway, macOS Seatbelt execution, approval fallback when no native sandbox exists, a Secure MCP Tunnel acceptance harness, and machine-readable resource benchmarks.

## Quick start

~~~bash
cargo build
cargo run -- doctor --workspace .
cargo run -- workspace check .
cargo run -- self-test --workspace .
cargo run -- benchmark --workspace . --iterations 10000
cargo run -- tunnel doctor --workspace .
cargo run -- serve --stdio --workspace .
~~~

## Documentation

The project uses mdBook for its documentation site.

~~~bash
cargo install mdbook
mdbook serve docs
~~~

GitHub Pages deployment is configured in .github/workflows/docs.yml.

## Architecture

~~~text
ChatGPT Web
    |
    | OpenAI Secure MCP Tunnel
    v
tunnel-client
    |
    | stdio MCP
    v
web-harness
    |
    v
Local repository
~~~

The full design blueprint lives in web-harness-final-architecture.md.

## Security

The remote model is never treated as local authority. Workspace reads/writes are canonical-path scoped, model-facing data is bounded, and process execution is argv-based with owned process groups. On macOS, execution uses a deny-by-default Seatbelt profile when `/usr/bin/sandbox-exec` is available; systems without a native backend require a one-time approval ticket.

Do not treat the current pre-1.0 release as a fully hardened remote-execution boundary: adversarial sandbox coverage, secret redaction, real remote Tunnel acceptance evidence, and complete Intel/Apple Silicon 8 GB evidence are still release gates.

See SECURITY.md and docs/src/security.md.

## Contributing

See CONTRIBUTING.md and AGENTS.md.

## License

Apache-2.0. See LICENSE, NOTICE, and THIRD_PARTY_NOTICES.md.

