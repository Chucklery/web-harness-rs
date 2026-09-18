# Installation

web-harness is designed to be distributed as one native binary. Rust, Node, Electron, and a browser runtime are not required on the end-user machine.

## GitHub Release

Tagged releases are configured to publish archives for:

- x86_64-unknown-linux-gnu
- x86_64-apple-darwin
- aarch64-apple-darwin

Each release includes per-archive SHA256 files plus an aggregate SHA256SUMS.

After downloading the archive for your platform:

~~~bash
tar -xzf web-harness-VERSION-TARGET.tar.gz
./web-harness version
~~~

Install it somewhere on PATH, for example:

~~~bash
mkdir -p ~/.local/bin
install -m 755 web-harness ~/.local/bin/web-harness
~~~

## Homebrew

The release workflow renders a version-specific Homebrew formula from the checked-in template and release checksums.

The canonical source repository is https://github.com/Chucklery/web-harness-rs. Until a dedicated Homebrew tap is published, download the generated formula from the release and install it explicitly:

~~~bash
brew install --formula ./web-harness.rb
~~~

A dedicated Homebrew tap is not published yet; the release formula remains the supported Homebrew installation path for now.

## From source

Requirements:

- Rust 1.80 or newer
- Git
- macOS or Linux

~~~bash
git clone https://github.com/Chucklery/web-harness-rs.git
cd web-harness-rs
cargo build --release
~~~

The binary is target/release/web-harness.

## Next step

After installation:

~~~bash
cd /path/to/project
web-harness setup \
  --workspace . \
  --tunnel-wrapper /absolute/path/to/tunnel-wrapper

web-harness connect
~~~

See Quick Start for the daily workflow.

## Documentation tooling

The docs site uses mdBook:

~~~bash
cargo install mdbook
mdbook serve docs
~~~

mdBook is only needed by documentation contributors.
