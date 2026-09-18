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

## Current project status

The local stdio server skeleton exists, but full ChatGPT-to-local end-to-end tunnel acceptance testing is still pending.

