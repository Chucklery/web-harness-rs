# Changelog

All notable changes will be documented here.

## Unreleased

### Changed

- Release packaging now consumes OpenAI's official `tunnel-client-runtime` artifact instead of the full tunnel-client distribution.
- Release/Homebrew bundles exclude cloudflared and the full tunnel-client CLI surface, while retaining required upstream license and SPDX evidence.
- Default production builds now exclude maintainer-only benchmark/release-gate code behind an opt-in `release-tools` feature.
- Release code generation is tuned for minimum size and Clap is built without unnecessary default UI features.
- Completed background Job history is bounded and evicts stale log artifacts instead of growing for the lifetime of the MCP process.
- Production atomic writes no longer depend on `tempfile`; that crate is restricted to tests and opt-in release tooling.
- Maintainer-only benchmark/release-gate sources now live under `src/maintenance/`, keeping the runtime module tree focused on shipped functionality.
- GitHub Releases now publish only five platform archives plus one aggregate `SHA256SUMS`; Homebrew metadata and per-target checksum intermediates are not release attachments.

## 0.3.0 - 2026-09-18

### Added

- Windows 10/11 x64 and ARM64 build/release targets with native ZIP packaging.
- Cross-platform hidden API-key input and Windows user credential storage.
- Explicit Apple Silicon release coverage for M1/M2/M3/M4 Macs.
- setup --show now reports the resolved tunnel-client path for installation diagnostics.

### Changed

- Default setup/connect no longer creates or depends on a generated zsh tunnel wrapper; web-harness launches the official tunnel-client directly.
- tunnel-client resolution now covers PATH, release bundles, Homebrew Cellar/opt layouts, Intel /usr/local, and Apple Silicon /opt/homebrew.
- OpenAI tunnel-client downloads now use persistent.oaistatic.com as the primary source with GitHub Releases as a fallback, with the same pinned SHA256 verification.
- Homebrew packaging preserves the complete libexec/web-harness directory layout.

### Fixed

- Fixed bundled OpenAI tunnel-client was not found on Intel macOS/Homebrew installations.

## 0.2.0 - 2026-09-18

### Added

- Interactive tunnel setup with hidden API-key input, managed zsh configuration, automatic tunnel wrapper generation, and safe setup status display.
- Official OpenAI tunnel-client v0.0.14 bundled into GitHub Release and Homebrew distributions with pinned SHA256 verification and upstream license/SPDX material.

### Changed

- setup now points users directly to OpenAI Platform Tunnels and Runtime API Keys before credential entry.
- connect resolves and launches the bundled tunnel-client automatically; release/Homebrew users no longer install tunnel-client separately.
- Release archives preserve the bundled OpenAI runtime under libexec/web-harness/.

### Removed

- Obsolete root-level web-harness-final-architecture.md; maintained architecture information now lives in the mdBook Architecture page and ADRs.

## 0.1.0 - 2026-09-18

### Added

- Initial Rust CLI and MCP stdio skeleton.
- Workspace canonicalization and path guard.
- Bounded batch file reading.
- Ripgrep-backed search and scoped AGENTS discovery.
- Structured Codex-style workspace patching.
- Bounded foreground/background execution and JobManager.
- Structured read-only Git gateway.
- One-time cryptographically bound execution approvals.
- Local self-test, kernel microbenchmark, and MCP stdio end-to-end test.
- macOS Seatbelt execution with workspace-scoped writes and approval fallback.
- Secure MCP Tunnel acceptance harness with externally injected official command.
- Machine-readable 8 GB benchmark reports and physical Intel Mac evidence.
- Release packaging, checksums, multi-target GitHub Release workflow, and Homebrew formula template.
- Secret redaction and minimized child-process environment inheritance.
- Hardened command policy and macOS Seatbelt adversarial tests.
- Optional Tunnel PID RSS sampling and explicit multi-machine release-gate aggregation.
- User-facing setup/connect/status/disconnect lifecycle with stale PID recovery.
- Approval-gated structured Git add/commit/switch/restore/push workflow.
- Open-source project baseline.
- mdBook documentation site and GitHub Pages workflow.

