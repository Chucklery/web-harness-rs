# web-harness

web-harness is a lightweight local Codex-style execution host designed to let ChatGPT Web work with a real local repository through MCP while keeping the local runtime small.

The project is intentionally not a second agent runtime. ChatGPT remains the agent; web-harness provides local execution capabilities.

Status: early bootstrap. The current implementation provides a Rust CLI, MCP stdio skeleton, workspace validation, a path guard, workspace_info, and bounded read_files. Patch, exec, jobs, Git, sandbox, approvals, and tunnel end-to-end validation remain roadmap work.

## Quick start

~~~bash
cargo build
cargo run -- doctor --workspace .
cargo run -- workspace check .
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

The remote model is never treated as local authority. Current protections are limited to workspace canonicalization/path guarding and bounded reads. The complete sandbox and approval engine described in the architecture document is not implemented yet.

See SECURITY.md and docs/src/security.md.

## Contributing

See CONTRIBUTING.md and AGENTS.md.

## License

Apache-2.0. See LICENSE, NOTICE, and THIRD_PARTY_NOTICES.md.

