# Installation

## From source

Requirements:

- Rust 1.80 or newer
- Git
- macOS or Linux for the initial development target

~~~bash
git clone YOUR_REPOSITORY_URL
cd web-harness-rs
cargo build --release
~~~

The resulting binary is under target/release/web-harness.

## Documentation tooling

The docs site uses mdBook:

~~~bash
cargo install mdbook
mdbook serve docs
~~~

mdBook is a documentation build dependency only. It is not part of the runtime path.

