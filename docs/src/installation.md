# Installation

web-harness ships as one release bundle: the harness binary plus the official OpenAI tunnel-client runtime under `libexec/web-harness/`. The `search` tool additionally uses the system [ripgrep](https://github.com/BurntSushi/ripgrep) (`rg`) binary. Homebrew installs ripgrep automatically; GitHub Release users install it separately. Rust, Node, Electron, a browser runtime, and a separate tunnel-client installation are not required on the end-user machine.

## GitHub Release

Tagged releases are configured to publish archives for:

- x86_64-unknown-linux-gnu
- x86_64-apple-darwin
- aarch64-apple-darwin
- x86_64-pc-windows-msvc
- aarch64-pc-windows-msvc

Each release includes per-archive SHA256 files plus an aggregate SHA256SUMS. Every platform archive also contains the matching official OpenAI tunnel-client runtime and its upstream license/SPDX material under libexec/web-harness/.

Windows releases are ZIP archives containing web-harness.exe and the matching bundled OpenAI tunnel-client.exe runtime under libexec/web-harness/.

After downloading the archive for your platform:

~~~bash
tar -xzf web-harness-VERSION-TARGET.tar.gz
./web-harness version
~~~

Install web-harness on PATH and preserve the bundled libexec directory, for example:

~~~bash
mkdir -p ~/.local/bin
install -m 755 web-harness ~/.local/bin/web-harness
mkdir -p ~/.local/libexec/web-harness
cp -R libexec/web-harness/. ~/.local/libexec/web-harness/
~~~

The `search` runtime requires ripgrep in `PATH`. Install it separately:

~~~bash
# macOS
brew install ripgrep

# Debian/Ubuntu
sudo apt install ripgrep

# Fedora
sudo dnf install ripgrep
~~~

Without ripgrep the `search` tool returns a clear dependency error; every other tool keeps working.

## Homebrew

The official tap is:

https://github.com/Chucklery/homebrew-tap

Install directly:

~~~bash
brew install Chucklery/tap/web-harness
~~~

Equivalent two-step form:

~~~bash
brew tap Chucklery/tap
brew install web-harness
~~~

The release workflow also renders a version-specific Homebrew formula from the checked-in template and release checksums.

The same formula supports both Intel Homebrew under /usr/local and Apple Silicon Homebrew under /opt/homebrew.

The formula declares `depends_on "ripgrep"`, so Homebrew installs the `search` dependency automatically.

## From source

Requirements:

- Rust 1.80 or newer
- Git
- macOS, Linux, or Windows

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
web-harness setup

web-harness connect
~~~

setup points to https://platform.openai.com/ for the OpenAI tunnel_id and runtime API key. On macOS/Linux it persists them in the managed zsh configuration block; on Windows it uses the current user's web-harness credentials.env file. The default flow launches the bundled official tunnel-client directly. No separate tunnel-client installation step is required.

See Quick Start for the daily workflow.

## Documentation tooling

The docs site uses mdBook:

~~~bash
cargo install mdbook
mdbook serve docs
~~~

mdBook is only needed by documentation contributors.
