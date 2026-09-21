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
- Git, sandbox, approval, and job primitives

The host does not run another model and does not embed a second agent loop.

There is exactly one external runtime dependency: the `search` tool invokes the system ripgrep binary instead of linking an embedded searcher. This keeps the release archive small and preserves ripgrep's own performance characteristics, at the cost of requiring `rg` in `PATH`. The dependency is declared in the Homebrew formula and reported by the search tool itself when it is missing; the rest of the harness has no external runtime requirements beyond the bundled tunnel-client.

## Tool Runtime boundary

MCP transport is being separated from local tool execution through a small in-process Tool Runtime. The MCP layer remains responsible for protocol parsing, response envelopes, and transport-specific error mapping; runtime tools own bounded workspace operations.

The `RuntimeTool` interface and `RuntimeRegistry` now dispatch all ten direct tools: workspace info/instructions, file reads, bounded file listing, search, patch, exec, job control, Git, and approval-ticket administration. Direct MCP calls and the adaptive-runtime compatibility gateway share the same registry path.

This boundary adds no second process, model loop, daemon, or database. Its only external runtime dependency is the system ripgrep binary used by `search`.

### ExecutionContext and capabilities

Runtime calls receive one `ExecutionContext` containing the workspace boundary, sandbox backend, permission engine, bounded limits, non-secret platform metadata, and Job manager access. This keeps security and resource policy out of individual MCP handlers.

Permissions use explicit capabilities such as `workspace.read`, `workspace.write`, `process.execute`, `job.control`, `git.read`, `git.local.write`, and `git.remote.write`. Approval tickets are cryptographically bound to the requested capability as well as the exact command payload, so an approval cannot be replayed for a stronger capability.

Ticket consumption happens only on a successful authorization. Expiry invalidates a ticket; denial removes it explicitly. A mismatching request — wrong capability, wrong argv, wrong cwd, or wrong background mode — fails without consuming the ticket, because a rejected request has not used the approval it was issued for. A retry with the correct payload therefore still succeeds within the five-minute window.

Execution network policy is part of the same approval digest. The default is deny; an outbound-only Seatbelt profile requires a distinct one-time approval, and systems without a native backend reject that upgrade rather than silently running unsandboxed.

Sandbox state is tracked as three distinct facts rather than a single boolean: whether a native backend is *available*, whether a sandbox is *already enforced* on the current process, and whether the current call *needs wrapping*. A process that is already inside a web-harness Seatbelt profile is still reported as sandboxed even though no second `sandbox-exec` is applied — nested application is what macOS rejects. Availability and enforcement are never collapsed into one flag.

Git has its own runtime policy and never routes through the generic exec sandbox. Read-only operations (`status`, paged `diff`, `log`, `show`, and revision-scoped `show_file`) use `git.read`; local mutations (`add`, `commit`, `switch`, `create_branch`, `restore`) use `git.local.write`; `push` uses `git.remote.write`. Local and remote mutations require distinct one-time approval-bound command payloads. Mutation approvals may also fence the repository with `expected_head`, and commit pathspecs prevent unrelated staged changes from being included.

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
