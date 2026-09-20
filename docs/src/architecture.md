# Architecture

The intended data path is:

~~~text
ChatGPT Web
    |
    v
OpenAI Secure MCP Tunnel
    |
    v
tunnel-client
    |
    | stdio
    v
web-harness
    |
    v
local workspace
~~~

## Responsibility split

ChatGPT owns:

- conversation
- reasoning
- planning
- tool orchestration

web-harness owns:

- workspace boundaries
- local file access
- local process lifecycle
- future Git, sandbox, approval, and job primitives

The host does not run another model and does not embed a second agent loop.

## Tool Runtime boundary

MCP transport is being separated from local tool execution through a small in-process Tool Runtime. The MCP layer remains responsible for protocol parsing, response envelopes, and transport-specific error mapping; runtime tools own bounded workspace operations.

The `RuntimeTool` interface and `RuntimeRegistry` now dispatch all nine direct tools: workspace info/instructions, file reads, search, patch, exec, job control, Git, and approval-ticket administration. Direct MCP calls and the adaptive-runtime compatibility gateway share the same registry path.

This boundary adds no second process, model loop, daemon, database, or new runtime dependency.

### ExecutionContext and capabilities

Runtime calls receive one `ExecutionContext` containing the workspace boundary, sandbox backend, permission engine, bounded limits, non-secret platform metadata, and Job manager access. This keeps security and resource policy out of individual MCP handlers.

Permissions use explicit capabilities such as `workspace.read`, `workspace.write`, `process.execute`, `job.control`, `git.read`, `git.local.write`, and `git.remote.write`. Approval tickets are cryptographically bound to the requested capability as well as the exact command payload, so an approval cannot be replayed for a stronger capability.

Git has its own runtime policy and never routes through the generic exec sandbox. Read-only operations (`status`, `diff`, `log`, `show`) use `git.read`; local mutations (`add`, `commit`, `switch`, `restore`) use `git.local.write`; `push` uses `git.remote.write`. Local and remote mutations require distinct one-time approval-bound command payloads.

Runtime status and tool manifest are derived from `ExecutionContext` and `RuntimeRegistry` rather than duplicate MCP constants. The MCP module is therefore limited to JSON-RPC/MCP envelopes, the four adaptive compatibility control calls, and transport-specific error-code mapping.

## Why a small local surface

The dominant end-to-end latency is expected to come from remote round trips, not a few local microseconds. Therefore the design prioritizes batch operations, compact schemas, compact results, and bounded local state.

Accepted architectural constraints are maintained in this documentation site and the ADRs under docs/src/adr/.

## Adaptive Runtime compatibility

Some ChatGPT Codex connector clients expect a small control surface before calling workspace tools. web-harness implements a compatibility shim for `runtime_status`, `work_on_project`, `tool_manifest`, and `call_runtime_tool`.

This is not a second agent runtime. `work_on_project` can only bind to the single workspace already configured when the stdio server starts, and `call_runtime_tool` can only dispatch to the existing bounded workspace/file/search/patch/exec/job/Git/permission tools. The shim does not add a database, project daemon, workflow engine, model client, or additional filesystem authority.

## Lightweight composition rules

The default binary contains only the normal setup/connect/MCP execution path. Low-frequency maintainer instrumentation such as machine benchmarks and release-gate aggregation is compile-time optional behind the `release-tools` feature and is not shipped in ordinary release archives.

Source layout follows the same boundary: normal runtime modules remain directly under `src/`, while opt-in benchmark and release validation code lives under `src/maintenance/`.

Runtime state must remain bounded. Background execution limits both concurrently running Jobs and retained completed Job metadata/output; old completed Jobs are evicted together with their temporary log artifacts. New capabilities should prefer the same pattern: one canonical runtime path, bounded state, and opt-in compilation for functionality normal users do not need.

