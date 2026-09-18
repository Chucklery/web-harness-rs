# Contributing

Thanks for contributing to web-harness.

## Development setup

Requirements:

- Rust toolchain compatible with the rust-version in Cargo.toml
- git
- ripgrep for future search work
- mdbook for local docs development

~~~bash
cargo fmt --check
cargo check
cargo test
~~~

Docs:

~~~bash
mdbook serve docs
~~~

## Pull requests

Keep changes focused and document:

- behavior changed
- tests added
- security impact
- dependency impact
- binary or RSS impact when adding runtime dependencies

Do not describe roadmap-only features as implemented.

