# ChatGPT Web and OpenAI Secure MCP Tunnel

The intended production connection is OpenAI Secure MCP Tunnel with web-harness running as a local stdio MCP server. Official web-harness release bundles include the matching OpenAI tunnel-client runtime.

Conceptually:

~~~text
ChatGPT Web -> Secure MCP Tunnel -> tunnel-client -> web-harness serve --stdio
~~~

## Important compatibility note

Tunnel CLI flags, authentication flows, ChatGPT settings, and developer-mode UI can change. Treat examples in this project as orientation, not as a substitute for the current OpenAI documentation.

Always verify setup against:

https://developers.openai.com/api/docs/guides/secure-mcp-tunnels

setup directs users to:

- https://platform.openai.com/settings/organization/tunnels for tunnel_id
- https://platform.openai.com/settings/organization/api-keys for the runtime API key

## Local command

The local server command that a tunnel profile launches is equivalent to:

~~~bash
web-harness serve --stdio --workspace /absolute/path/to/project
~~~

The host does not need a public listening port when used through a stdio tunnel setup.

## Acceptance harness

Validate the local stdio MCP contract first:

~~~bash
web-harness tunnel doctor --workspace /path/to/project
~~~

This performs initialize/tools-list and bounded read-only `tools/call` roundtrips for workspace info, file listing, search, and Git status against a locally spawned web-harness server. It validates the normal tool-result envelope without modifying the configured workspace.

For a real Secure MCP Tunnel acceptance run, inject the exact current official command as a JSON argv array instead of hard-coding tunnel-client flags in this repository:

~~~bash
export WEB_HARNESS_TUNNEL_COMMAND_JSON='["/path/to/your-acceptance-wrapper"]'
web-harness tunnel accept --workspace /path/to/project
~~~

The wrapper receives:

- WEB_HARNESS_SERVER_BIN
- WEB_HARNESS_SERVER_ARGS_JSON
- WEB_HARNESS_WORKSPACE

It is responsible for invoking the current official Secure MCP Tunnel flow and validating the remote side. To count as passed, it must exit 0 and write exactly one JSON acceptance-evidence object to stdout; diagnostic logs belong on stderr. A zero exit code alone is not acceptance evidence.

Evidence is bounded to 16 KiB and contains no arbitrary text or workspace contents. It must include `schema_version: 1`, bounded `client_version` and `protocol_version` tokens, each passing stage exactly once in order, contiguous tool-call records, and a `failure_recovery` value. Required stage names are `connect_workspace`, `discover_project`, `read_instructions_and_source`, `search_symbol`, `modify_workspace`, `run_validation`, `observe_background_job`, `inspect_git_changes`, `verify_user_approval`, and `disconnect_cleanup`. Required tool-call names are `workspace_info`, `list_files`, `workspace_instructions`, `read_files`, `search`, `patch`, `exec`, `job_wait`, `git_status`, and `git_diff`. Additional recognized names are `runtime_status`, `work_on_project`, `tool_manifest`, `call_runtime_tool`, `permission`, `job_poll`, `job_output`, `job_list`, `job_cancel`, `git_head`, `git_log`, `git_show`, `git_show_file`, `git_add`, `git_commit`, `git_switch`, `git_create_branch`, `git_restore`, and `git_push`. Each record contains a `sequence` starting at one, a recognized tool name, an `outcome` (`succeeded`, `approval_required`, `failed`, or `recovered`), and either `null` or a fixed machine-readable `error_code`. The evidence must show an approval challenge followed by a successful retry for the same tool. `failure_recovery` is one of `not_needed`, `retry_after_transient_failure`, `retry_after_host_reconnect`, `retry_after_approval_decline`, or `other_recovered`; each failed tool call must have a later recovered record for that tool, and at least one tool-call record must be marked `recovered` when a recovery path is recorded.

The allowed error codes are `invalid_arguments`, `not_found`, `denied`, `permission_denied`, `dependency_unavailable`, `execution_failed`, `conflict`, and `limit_exceeded`.

The report from `tunnel accept` includes the validated evidence. Preserve that report as acceptance metadata, but do not add credentials, raw tool arguments/results, source snippets, or other sensitive workspace data.

This split is deliberate: web-harness owns and verifies its local MCP contract, while the injected command tracks OpenAI's current tunnel CLI/UI without this project inventing flags that may change.
