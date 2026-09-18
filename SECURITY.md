# Security Policy

## Supported versions

This project is pre-1.0. Security fixes target the latest branch or release unless otherwise stated.

## Reporting a vulnerability

Use GitHub private security advisories when enabled. Avoid filing public issues for exploitable vulnerabilities or secrets.

## Current security boundary

The current pre-1.0 implementation includes:

- canonical workspace root resolution
- rejection of canonical paths outside the workspace
- symlink escape protection through canonicalization
- bounded UTF-8 file reads
- structured workspace patching with path guards and atomic writes
- argv-based process execution with bounded returned output
- owned Unix process groups and bounded background jobs
- one-time SHA-256-bound execution approval tickets
- structured read-only Git operations
- macOS Seatbelt execution when /usr/bin/sandbox-exec is available, with deny-by-default policy and workspace-scoped writes
- secret redaction on process and tunnel output for common token/password/key/cookie/authorization patterns
- minimized child-process environment via env_clear plus a small operational allowlist
- command policy rejects executable path traversal and direct host-control tools such as reboot/shutdown/disk-management privilege escalation entry points
- macOS adversarial sandbox coverage for network denial, workspace-outside write denial, workspace/tmp write allow, and system read allow

The following architecture components are not yet implemented:

- broader adversarial coverage across more developer toolchains and shell compositions

Do not treat the current pre-1.0 implementation as a fully hardened remote-execution boundary until native OS sandbox enforcement is complete.

