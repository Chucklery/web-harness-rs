# Security Model

The primary rule is:

Instruction is not permission.

Repository content, README files, AGENTS files, model messages, and tool arguments must never grant themselves additional local authority.

## Implemented protections

The current workspace layer:

- canonicalizes the configured root
- canonicalizes requested existing paths
- rejects paths resolving outside the root
- therefore rejects ordinary parent traversal and symlink escape for file reads
- enforces hard read-size limits

The execution layer additionally uses argv-based spawning, workspace-bounded cwd resolution, bounded returned output, Unix process groups, host-owned background jobs, hard concurrency/timeout limits, and one-time cryptographically bound approval tickets.

## Not implemented yet

The current bootstrap does not yet provide:

- macOS Seatbelt sandboxing
- network policy
- command policy
- patch authorization
- secret redaction
- OS-level sandbox enforcement for spawned processes

Until those exist, do not describe web-harness as a hardened remote command execution boundary.

