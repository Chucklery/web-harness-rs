# Release Process

The project is pre-1.0. A release candidate should not be cut until:

- cargo fmt --check passes
- cargo check passes
- cargo test passes
- docs build successfully
- security-sensitive changes were reviewed
- third-party notices are current
- CHANGELOG is updated

Before a stable release, also require:

- ChatGPT Web tunnel end-to-end acceptance
- native OS sandbox enforcement and adversarial sandbox coverage
- low-memory benchmarks on 8 GB Intel and Apple Silicon Macs

Use the machine-readable release gate before tagging:

~~~bash
web-harness release-gate \
  --evidence benchmarks/intel.json \
  --evidence benchmarks/apple-silicon.json
~~~

not_evaluated means evidence is incomplete and is not equivalent to a release pass.

Publishing and GitHub release creation are intentionally separate from CI validation.

## Release artifacts

The tag-triggered `.github/workflows/release.yml` builds locked release binaries for Linux x86_64, macOS Intel, and macOS Apple Silicon.

Each target is packaged with:

- web-harness
- LICENSE
- NOTICE
- README.md

The workflow produces per-archive SHA256 files and an aggregate `SHA256SUMS`, then renders a Homebrew formula from `packaging/homebrew/web-harness.rb.template`.

## Local packaging check

Use the same packaging script as CI:

~~~bash
host_target="$(rustc -vV | sed -n 's/^host: //p')"
cargo build --release --locked --target "$host_target"
./scripts/package-release.sh "$host_target" 0.1.0
~~~

The Homebrew renderer expects checksums for all release targets:

~~~bash
./scripts/render-homebrew-formula.sh 0.1.0 dist dist/web-harness.rb
~~~

## Publishing

The workflow only runs for a pushed `v*` tag. Normal CI, local builds, and merges do not publish releases.

Creating or pushing a tag is a maintainer action and should happen only after the release gates above are satisfied.

