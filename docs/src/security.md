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

For first-use convenience, interactive setup can persist CONTROL_PLANE_TUNNEL_ID and CONTROL_PLANE_API_KEY in a web-harness-managed block in ~/.zshrc (or ZDOTDIR/.zshrc). API-key input disables terminal echo; the API key is not copied into config.json or the generated tunnel wrapper. The shell file is rewritten atomically with mode 0600.

This is still plaintext credential storage. A local process or user that can read the shell file can recover the API key. Non-interactive --api-key is also less private because command-line arguments may enter shell history or be visible to local process inspection. Users with stronger secret-management requirements should use a custom wrapper or another external secret store.

## Bundled tunnel-client supply chain

Official release/Homebrew packages redistribute the upstream OpenAI tunnel-client release payload rather than a locally modified fork. The packaging script pins the upstream tunnel-client version and platform ZIP SHA256 before extraction. The upstream LICENSE, NOTICE, dependency-license report, SPDX manifest, cloudflared manifest, and matching runtime binaries remain together under libexec/web-harness/.

## Command policy

The exec path applies a lightweight policy before process creation. It rejects executable paths containing parent traversal and direct host-control executables such as shutdown/reboot, disk-management/formatting tools, and privilege-escalation front doors. It intentionally does not ban ordinary developer commands such as git, cargo, or file deletion inside the sandbox because the workspace/sandbox boundary is the primary authority boundary.

The macOS test suite exercises four concrete Seatbelt properties: workspace-outside writes are denied, local network connections are denied, workspace and temporary writes are allowed, and system files/tools remain readable.

## Current limitations

The current pre-1.0 boundary still has known limits:

- non-macOS platforms do not yet have a native sandbox backend and therefore rely on explicit approvals
- secret redaction is heuristic and cannot recognize every possible secret format
- broader sandbox compatibility testing is still needed across more developer toolchains and shell compositions
- real production Secure MCP Tunnel acceptance evidence remains outstanding

Do not treat pre-1.0 as a universally hardened remote-execution boundary across every platform and toolchain.
