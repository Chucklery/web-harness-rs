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

The following architecture components are not yet implemented:

- macOS sandbox enforcement
- secret redaction
- adversarial sandbox coverage and a hardened command policy

Do not treat the current pre-1.0 implementation as a fully hardened remote-execution boundary until native OS sandbox enforcement is complete.

