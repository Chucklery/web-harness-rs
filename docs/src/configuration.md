# Configuration

web-harness keeps configuration deliberately small and does not store authentication secrets.

## User configuration

setup writes JSON configuration to:

- XDG_CONFIG_HOME/web-harness/config.json when XDG_CONFIG_HOME is set
- otherwise HOME/.config/web-harness/config.json

The current schema stores:

- schema version
- a default/setup workspace
- optional tunnel wrapper argv

Example setup:

~~~bash
web-harness setup \
  --workspace . \
  --tunnel-wrapper /opt/bin/my-tunnel-wrapper
~~~

For the common case, --tunnel-wrapper stores one absolute wrapper path.

Advanced users can instead use --tunnel-command-json with a bounded JSON argv array. It is still argv, not a shell string.

web-harness rejects command arguments that appear to contain tokens, passwords, API keys, cookies, bearer credentials, or similar secrets.

## Runtime state

Runtime state is written under:

- XDG_STATE_HOME/web-harness when XDG_STATE_HOME is set
- otherwise HOME/.local/state/web-harness

It contains only bounded non-secret connection metadata such as the owned tunnel PID and active workspace.

Normal connect does not persist tunnel stdout/stderr.

## Workspace selection

setup records a workspace for initial configuration and disconnected status display.

Daily connect uses the current directory by default:

~~~bash
cd project-a
web-harness connect

web-harness disconnect

cd ../project-b
web-harness connect
~~~

Use --workspace to override the current directory explicitly.

## Tunnel wrapper contract

The configured wrapper receives three environment variables:

- WEB_HARNESS_SERVER_BIN
- WEB_HARNESS_SERVER_ARGS_JSON
- WEB_HARNESS_WORKSPACE

The wrapper is responsible for invoking the current official OpenAI Secure MCP Tunnel flow and launching/attaching the provided stdio MCP server as required by that flow.

This indirection is intentional because official tunnel CLI flags and UI can evolve independently from web-harness.
