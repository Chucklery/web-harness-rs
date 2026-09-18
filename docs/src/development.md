# Development

## Principles

- keep the runtime dependency budget small
- avoid default daemons, indexers, LSPs, browsers, and Node runtimes
- keep business logic independent from MCP framing where practical
- add hard limits for model-facing input and output
- preserve unrelated user changes

## Standard loop

~~~bash
cargo fmt --check
cargo check
cargo test
~~~

For docs:

~~~bash
mdbook build docs
~~~

Project-level contributor instructions are in AGENTS.md.

