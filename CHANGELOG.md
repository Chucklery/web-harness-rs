# Changelog

All notable changes will be documented here.

## Unreleased

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

