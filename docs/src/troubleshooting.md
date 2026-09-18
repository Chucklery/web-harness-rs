# Troubleshooting

## web-harness says it is not configured

Run setup first:

~~~bash
web-harness setup
~~~

Official release and Homebrew packages include tunnel-client. If setup says it is unavailable, first run `web-harness setup --show` and inspect the reported `tunnel-client path`. v0.3+ searches PATH, release-bundle libexec paths, Homebrew Cellar/opt layouts, Intel `/usr/local`, and Apple Silicon `/opt/homebrew`. If no path is resolved, reinstall the matching web-harness package. Source builds may use a developer-provided tunnel-client on PATH or WEB_HARNESS_TUNNEL_CLIENT_BIN.

## I changed the key but connect still fails

Run interactive setup again. On macOS/Linux web-harness replaces its managed ~/.zshrc block rather than appending duplicates; Windows updates the current user's credentials.env block:

~~~bash
web-harness setup
web-harness setup --show
~~~

connect reads the managed credential file directly, so you do not need to open a new terminal or source ~/.zshrc.

## Where is my API key stored?

On macOS/Linux it is plaintext in the web-harness-managed block in ~/.zshrc (or ZDOTDIR/.zshrc). On Windows it is plaintext in `%APPDATA%\\web-harness\\credentials.env` when APPDATA is available. It is not stored in config.json or a tunnel wrapper.

On Unix the shell file is rewritten with mode 0600. Backups created by web-harness omit the managed secret block.

## connect cannot find tunnel-client on Intel macOS 13

Use:

~~~bash
web-harness setup --show
brew --prefix
brew list web-harness
~~~

For Homebrew installs, v0.3+ resolves both the real Cellar executable path and the `/usr/local/opt/web-harness/libexec/web-harness/tunnel-client` layout used by Intel Homebrew. Reinstalling v0.2 does not fix the old resolver; use a v0.3+ build.

## OpenAI/GitHub tunnel-client download fails during packaging

The packaging scripts use `https://persistent.oaistatic.com/tunnel-client/` as the primary official download source and GitHub Releases only as a fallback. Both paths are pinned to the same upstream version and SHA256. A checksum mismatch always fails packaging instead of silently accepting another binary.

## connect says the tunnel command is not configured

Run setup again with the wrapper command, or provide a one-off override:

~~~bash
web-harness connect \
  --tunnel-command-json '["/absolute/path/to/tunnel-wrapper"]'
~~~

The override is not written back to configuration.

## connect starts the wrong repository

connect uses the current directory by default.

~~~bash
cd /path/to/correct/project
web-harness connect
~~~

Or use:

~~~bash
web-harness connect --workspace /path/to/project
~~~

## status reports stale state recovered

The previously recorded tunnel PID no longer exists. web-harness removes stale state automatically and reports the recovery.

Run connect again.

## the tunnel process exits immediately

Normal connect intentionally discards tunnel stdout/stderr so credentials or authentication output are not persisted.

Use the bounded diagnostic commands instead:

~~~bash
web-harness tunnel doctor --workspace .
web-harness tunnel accept --workspace . --command-json '["/path/to/acceptance-wrapper"]'
~~~

Their returned output is bounded and passed through secret redaction.

## workspace check fails

Confirm the path exists and is a directory:

~~~bash
web-harness workspace check /absolute/path/to/project
~~~

## a file read or Git pathspec is rejected

Typical causes:

- the canonical path escapes the workspace
- a symlink resolves outside the workspace
- a file exceeds a read limit
- the path is not valid for the requested structured Git operation

## a Git mutation did not run

add, commit, switch, restore, and push require an approval ticket. The first call returns approval_required. Approve that ticket, then retry the exact same Git operation with its approval_id.

A ticket cannot authorize a different mutation.

## MCP output is corrupted

The MCP protocol uses stdout. Diagnostic logging must go to stderr. Avoid printing unrelated data to stdout while running serve --stdio.

## Secure MCP Tunnel does not connect

First run tunnel doctor to verify the local stdio MCP contract. Then use setup --show to verify that tunnel_id, API-key presence, the resolved bundled tunnel-client path, and the selected workspace are correct. The default path launches the official client directly with the control-plane environment variables and stdio MCP command binding.
