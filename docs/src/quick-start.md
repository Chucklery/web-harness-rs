# Quick Start

The normal user path is setup once, then connect from whichever repository you want ChatGPT to work on.

## 1. Configure the tunnel wrapper

~~~bash
cd /path/to/one/project

web-harness setup \
  --workspace . \
  --tunnel-wrapper /absolute/path/to/tunnel-wrapper
~~~

The wrapper configuration must not contain credentials. Use the official tunnel login state or environment instead.

The wrapper receives WEB_HARNESS_SERVER_BIN, WEB_HARNESS_SERVER_ARGS_JSON, and WEB_HARNESS_WORKSPACE.

## 2. Connect a repository

~~~bash
cd /path/to/project
web-harness connect
~~~

connect uses the current directory by default. To override it explicitly:

~~~bash
web-harness connect --workspace /path/to/project
~~~

The command starts the configured tunnel wrapper in its own owned process group and returns immediately.

## 3. Check status

~~~bash
web-harness status
~~~

Status shows the active workspace, tunnel PID, native sandbox status, and whether stale process state was recovered.

## 4. Use ChatGPT Web

Once the OpenAI Secure MCP Tunnel side is connected, use ChatGPT normally. Typical requests include reading project instructions, changing files, running tests, reviewing diffs, staging files, committing, and pushing.

Git mutations always require an explicit one-time approval. Push is marked as a higher-risk remote mutation.

## 5. Disconnect

~~~bash
web-harness disconnect
~~~

disconnect terminates the owned tunnel process group and removes the runtime state file.

## Advanced diagnostics

~~~bash
web-harness doctor --workspace .
web-harness self-test --workspace .
web-harness tunnel doctor --workspace .
web-harness benchmark --workspace . --iterations 10000
web-harness serve --stdio --workspace .
~~~
