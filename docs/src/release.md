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
cargo run --release --features release-tools -- release-gate \
  --evidence benchmarks/intel.json \
  --evidence benchmarks/apple-silicon.json
~~~

not_evaluated means evidence is incomplete and is not equivalent to a release pass.

Publishing and GitHub release creation are intentionally separate from CI validation.

## Release artifacts

The tag-triggered `.github/workflows/release.yml` first runs formatting, default/release-tools checks and tests, plus mdBook validation. Only after that gate passes does it build locked release binaries for Linux x86_64, macOS Intel, macOS Apple Silicon, Windows x64, and Windows ARM64.

Each target is packaged with:

- web-harness
- the pinned official OpenAI `tunnel-client-runtime` binary plus required upstream legal/SPDX evidence under libexec/web-harness/
- LICENSE
- NOTICE
- README.md
- THIRD_PARTY_NOTICES.md

Each platform archive is extracted and smoke-tested before upload. The packaged binary must report the tagged version, expose normal help/setup behavior, contain the runtime-only tunnel-client, exclude cloudflared, and exclude maintainer-only commands. The publish job requires exactly five platform archives, verifies them, and publishes one aggregate `SHA256SUMS` alongside those archives.

Release assets intentionally exclude CI intermediates and package-manager metadata. Homebrew packaging is maintained separately from the GitHub Release attachment set.

## Local packaging check

Use the same packaging script as CI:

~~~bash
host_target="$(rustc -vV | sed -n 's/^host: //p')"
cargo build --release --locked --target "$host_target"
./scripts/package-release.sh "$host_target" 0.3.2
~~~

Release archives intentionally use the default feature set. Maintainer-only benchmark and release-gate code is compiled only when `--features release-tools` is requested and is not shipped in normal archives.

The Homebrew renderer expects checksums for all release targets:

~~~bash
./scripts/render-homebrew-formula.sh 0.3.2 dist dist/web-harness.rb
~~~

## Publishing

The workflow only runs for a pushed `v*` tag. Normal CI, local builds, and merges do not publish releases.

Creating or pushing a tag is a maintainer action and should happen only after the release gates above are satisfied.

