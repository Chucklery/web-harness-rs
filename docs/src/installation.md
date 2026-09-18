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

## From a GitHub Release

Tagged releases are configured to publish archives for:

- x86_64-unknown-linux-gnu
- x86_64-apple-darwin
- aarch64-apple-darwin

Each release includes per-archive SHA256 files plus an aggregate `SHA256SUMS`.

After downloading the archive that matches your machine:

~~~bash
tar -xzf web-harness-VERSION-TARGET.tar.gz
./web-harness version
~~~

Move the binary somewhere on PATH, for example:

~~~bash
install -m 755 web-harness ~/.local/bin/web-harness
~~~

## Homebrew formula asset

The release workflow also renders a version-specific `web-harness.rb` formula from the checked-in template and release checksums.

Until a dedicated Homebrew tap is created, the generated formula can be downloaded from a GitHub Release and installed explicitly:

~~~bash
brew install --formula ./web-harness.rb
~~~

The repository intentionally ships a formula template rather than hard-coding an owner/repository URL before the canonical public repository is fixed.

## Documentation tooling

The docs site uses mdBook:

~~~bash
cargo install mdbook
mdbook serve docs
~~~

mdBook is a documentation build dependency only. It is not part of the runtime path.

