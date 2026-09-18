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
- patch, exec, jobs, Git, sandbox, and approval coverage
- low-memory benchmarks on 8 GB Intel and Apple Silicon Macs

Publishing and GitHub release creation are intentionally separate from CI validation.

