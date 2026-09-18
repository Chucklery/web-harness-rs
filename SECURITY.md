# Security Policy

## Supported versions

This project is pre-1.0. Security fixes target the latest branch or release unless otherwise stated.

## Reporting a vulnerability

Use GitHub private security advisories when enabled. Avoid filing public issues for exploitable vulnerabilities or secrets.

## Current security boundary

The bootstrap release implements:

- canonical workspace root resolution
- rejection of canonical paths outside the workspace
- symlink escape protection through canonicalization
- bounded UTF-8 file reads

The following architecture components are not yet implemented:

- macOS sandbox enforcement
- structured command approval tickets
- execution policy
- secret redaction
- permission-aware patching
- process isolation

Do not treat the current bootstrap as a hardened remote-execution boundary.

