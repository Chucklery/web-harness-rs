# Configuration

web-harness keeps config.json deliberately small. In the convenience-first zsh flow, tunnel credentials are persisted separately in a managed ~/.zshrc block.

## User configuration

setup writes JSON configuration to:

- XDG_CONFIG_HOME/web-harness/config.json when XDG_CONFIG_HOME is set
- otherwise HOME/.config/web-harness/config.json

The current schema stores:

- schema version
- a default/setup workspace
- optional custom tunnel command argv

Normal setup is interactive:

~~~bash
web-harness setup
~~~

It prompts for tunnel_id and the runtime API key. The API-key prompt disables terminal echo.

web-harness writes a managed block:

~~~text
# >>> web-harness tunnel >>>
export CONTROL_PLANE_TUNNEL_ID='...'
export CONTROL_PLANE_API_KEY='...'
# <<< web-harness tunnel <<<
~~~

The target is ~/.zshrc, or ZDOTDIR/.zshrc when ZDOTDIR is set.

The update is idempotent: repeated setup replaces this block instead of appending duplicates. The file is atomically rewritten with mode 0600. Before an update, web-harness creates a private backup with the managed secret block removed so old API keys are not duplicated into backup files.

The default path does not generate a wrapper. connect reads the managed credential file directly, injects the two control-plane environment variables, resolves the bundled/installed OpenAI tunnel-client, and launches it directly. A new terminal or manual source ~/.zshrc is not required.

### Plaintext warning

This design stores the runtime API key in plaintext in ~/.zshrc for convenience. Anyone who can read that file can read the key. Users who prefer stronger secret storage should use the advanced custom-wrapper flow instead.

### Non-interactive setup

~~~bash
web-harness setup \
  --tunnel-id tunnel_0123456789abcdef0123456789abcdef \
  --api-key 'YOUR_RUNTIME_KEY'
~~~

Passing an API key on the command line is less private because shell history and local process inspection may expose it. Prefer interactive setup for normal use.

### Advanced wrapper override

Advanced users can still use --tunnel-wrapper or --tunnel-command-json. These remain useful when authentication is handled externally.

web-harness rejects secret-looking values embedded in --tunnel-command-json.

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

## Advanced custom tunnel command contract

The configured wrapper receives three environment variables:

- WEB_HARNESS_SERVER_BIN
- WEB_HARNESS_SERVER_ARGS_JSON
- WEB_HARNESS_WORKSPACE

The wrapper is responsible for invoking the current official OpenAI Secure MCP Tunnel flow and launching/attaching the provided stdio MCP server as required by that flow.

This indirection is intentional because official tunnel CLI flags and UI can evolve independently from web-harness.

The default launch path relies on the official CONTROL_PLANE_TUNNEL_ID and CONTROL_PLANE_API_KEY environment variables and invokes the resolved tunnel-client with the stdio MCP binding. Advanced users may still configure a custom wrapper/argv. Source builds may fall back to a developer-provided tunnel-client on PATH.
