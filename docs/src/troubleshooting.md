# Troubleshooting

## Workspace check fails

Confirm the path exists and is a directory:

~~~bash
web-harness workspace check /absolute/path/to/project
~~~

## File read is rejected

Typical causes:

- the path does not exist
- the canonical path escapes the workspace through a symlink
- the file exceeds the per-file limit
- the file is not valid UTF-8
- the batch exceeds the total read limit

## MCP output is corrupted

The MCP protocol uses stdout. Diagnostic logging must go to stderr. Avoid printing unrelated data to stdout while serving.

## Tunnel does not connect

First confirm web-harness works locally over stdio, then use the current OpenAI tunnel-client diagnostics and official Secure MCP Tunnel documentation.

