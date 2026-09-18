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

The execution layer additionally uses argv-based spawning, workspace-bounded cwd resolution, bounded returned output, Unix process groups, host-owned background jobs, and hard concurrency/timeout limits.

On macOS, when /usr/bin/sandbox-exec is available, process execution is wrapped in a deny-by-default Seatbelt profile. The profile allows process creation, read access required by normal tooling, and writes only inside the configured workspace and temporary directories; network access is not allowed by the profile. When no native sandbox backend is available, execution falls back to one-time cryptographically bound approval tickets.

## Secret handling

Child processes do not inherit the full web-harness environment. The host clears the environment before exec and rebuilds a small operational allowlist such as PATH, HOME, TMPDIR, locale variables, terminal metadata, and SSH_AUTH_SOCK. Variables whose names look like tokens, passwords, cookies, secrets, credentials, or API keys are not forwarded.

Process stdout/stderr and externally injected tunnel acceptance output are passed through a bounded redaction layer before being returned to the model or CLI. The redactor covers common assignment forms and bearer authorization values.

Redaction is a defense-in-depth measure, not permission to intentionally print secrets. Unknown secret formats can still exist, so commands should avoid emitting credentials in the first place.

## Command policy

The exec path applies a lightweight policy before process creation. It rejects executable paths containing parent traversal and direct host-control executables such as shutdown/reboot, disk-management/formatting tools, and privilege-escalation front doors. It intentionally does not ban ordinary developer commands such as git, cargo, or file deletion inside the sandbox because the workspace/sandbox boundary is the primary authority boundary.

The macOS test suite exercises four concrete Seatbelt properties: workspace-outside writes are denied, local network connections are denied, workspace and temporary writes are allowed, and system files/tools remain readable.

## Not implemented yet

The current bootstrap does not yet provide:

- macOS Seatbelt sandboxing
- network policy
- command policy
- patch authorization
- secret redaction
- hardened sandbox policy coverage for more toolchains and adversarial cases

Until those exist, do not describe web-harness as a hardened remote command execution boundary.

