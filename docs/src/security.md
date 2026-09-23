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

For first-use convenience, interactive setup persists CONTROL_PLANE_TUNNEL_ID and CONTROL_PLANE_API_KEY in a user credential file. On macOS/Linux this is a web-harness-managed block in ~/.zshrc (or ZDOTDIR/.zshrc), rewritten atomically with mode 0600. On Windows it is a plaintext credentials.env file under the current user's APPDATA when available. API-key input disables terminal echo, and the API key is not copied into config.json or tunnel command arguments.

This is still plaintext credential storage. A local process or user that can read the shell file can recover the API key. Non-interactive --api-key is also less private because command-line arguments may enter shell history or be visible to local process inspection. Users with stronger secret-management requirements should use a custom wrapper or another external secret store.

## Bundled tunnel-client supply chain

Official release/Homebrew packages redistribute OpenAI's narrow `tunnel-client-runtime` artifact rather than the full tunnel-client distribution or a locally modified fork. Packaging pins the upstream `SHA256SUMS.txt` digest, verifies the selected runtime ZIP against that manifest, and then copies only the runtime binary plus LICENSE, NOTICE, dependency-license report, and SPDX manifest under `libexec/web-harness/`. The full CLI and cloudflared companion are intentionally excluded.

## Command policy

The exec path applies a lightweight policy before process creation. It rejects executable paths containing parent traversal and direct host-control executables such as shutdown/reboot, disk-management/formatting tools, and privilege-escalation front doors. It intentionally does not ban ordinary developer commands such as git, cargo, or file deletion inside the sandbox because the workspace/sandbox boundary is the primary authority boundary.

The macOS test suite exercises four concrete Seatbelt properties: workspace-outside writes are denied, local network connections are denied, workspace and temporary writes are allowed, and system files/tools remain readable.

## Current limitations

The current pre-1.0 boundary still has known limits:

- Linux Landlock has only received an upstream-documentation feasibility review; it is not implemented or host-tested. ABI-dependent filesystem and network rights, plus operations such as `chmod` that Landlock cannot restrict, prevent us from claiming Seatbelt-equivalent enforcement. Non-macOS execution continues to rely on explicit approvals. See the [upstream Landlock userspace documentation](https://docs.kernel.org/userspace-api/landlock.html).
- secret redaction is heuristic and cannot recognize every possible secret format
- broader sandbox compatibility testing is still needed across more developer toolchains and shell compositions
- direct protected-path classification now covers common `.env`, private-key, and credential files for file reads, workspace instructions, listing/search filtering, Git revision-file reads and diffs, and recognizable exec path arguments. File reads, instruction discovery, search scopes, enumeration, and exec resolve in-workspace symlinks before classifying their targets, preventing aliases from bypassing these checks. Git diffs scan changed path names before returning content and require approval when protected files are included. A one-shot script containing recognizable Git mutation tokens additionally requires `git.local.write` or `git.remote.write` as appropriate; this text heuristic is not a complete audit of arbitrary scripts, which remain governed by explicit script approval and the OS sandbox.
- real production Secure MCP Tunnel acceptance evidence remains outstanding

Do not treat pre-1.0 as a universally hardened remote-execution boundary across every platform and toolchain.
