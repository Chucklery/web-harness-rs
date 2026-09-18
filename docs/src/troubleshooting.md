# Troubleshooting

## web-harness says it is not configured

Run setup first:

~~~bash
web-harness setup
~~~

If setup says tunnel-client is not found, install the official OpenAI tunnel-client and make sure tunnel-client is on PATH. web-harness intentionally does not guess an installation command.

## I changed the key but connect still fails

Run interactive setup again. web-harness replaces its managed ~/.zshrc block rather than appending duplicates:

~~~bash
web-harness setup
web-harness setup --show
~~~

connect reads the managed block directly, so you do not need to open a new terminal or source ~/.zshrc.

## Where is my API key stored?

In the default convenience flow it is plaintext in the web-harness-managed block in ~/.zshrc (or ZDOTDIR/.zshrc). It is not stored in config.json or the generated tunnel wrapper.

The shell file is rewritten with mode 0600. Backups created by web-harness omit the managed secret block.

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

## the tunnel wrapper exits immediately

Normal connect intentionally discards wrapper stdout/stderr so credentials or authentication output are not persisted.

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

First run tunnel doctor to verify the local stdio MCP contract. Then confirm tunnel-client is on PATH and use setup --show to verify that tunnel_id and API-key presence are configured. The generated wrapper uses the official control-plane environment variables and the stdio MCP command binding.
