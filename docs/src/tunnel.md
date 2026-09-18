# ChatGPT Web and OpenAI Secure MCP Tunnel

The intended production connection is OpenAI Secure MCP Tunnel with web-harness running as a local stdio MCP server.

Conceptually:

~~~text
ChatGPT Web -> Secure MCP Tunnel -> tunnel-client -> web-harness serve --stdio
~~~

## Important compatibility note

Tunnel CLI flags, authentication flows, ChatGPT settings, and developer-mode UI can change. Treat examples in this project as orientation, not as a substitute for the current OpenAI documentation.

Always verify setup against:

https://developers.openai.com/api/docs/guides/secure-mcp-tunnels

## Local command

The local server command that a tunnel profile should eventually launch is equivalent to:

~~~bash
web-harness serve --stdio --workspace /absolute/path/to/project
~~~

The host does not need a public listening port when used through a stdio tunnel setup.

## Acceptance harness

Validate the local stdio MCP contract first:

~~~bash
web-harness tunnel doctor --workspace /path/to/project
~~~

This performs initialize and tools/list roundtrips against a locally spawned web-harness server.

For a real Secure MCP Tunnel acceptance run, inject the exact current official command as a JSON argv array instead of hard-coding tunnel-client flags in this repository:

~~~bash
export WEB_HARNESS_TUNNEL_COMMAND_JSON='["/path/to/your-acceptance-wrapper"]'
web-harness tunnel accept --workspace /path/to/project
~~~

The wrapper receives:

- WEB_HARNESS_SERVER_BIN
- WEB_HARNESS_SERVER_ARGS_JSON
- WEB_HARNESS_WORKSPACE

It is responsible for invoking the current official Secure MCP Tunnel flow and validating the remote side. Exit code 0 marks the external acceptance step as passed.

This split is deliberate: web-harness owns and verifies its local MCP contract, while the injected command tracks OpenAI's current tunnel CLI/UI without this project inventing flags that may change.

