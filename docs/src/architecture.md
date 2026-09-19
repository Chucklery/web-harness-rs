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

## Why a small local surface

The dominant end-to-end latency is expected to come from remote round trips, not a few local microseconds. Therefore the design prioritizes batch operations, compact schemas, compact results, and bounded local state.

Accepted architectural constraints are maintained in this documentation site and the ADRs under docs/src/adr/.

## Lightweight composition rules

The default binary contains only the normal setup/connect/MCP execution path. Low-frequency maintainer instrumentation such as machine benchmarks and release-gate aggregation is compile-time optional behind the `release-tools` feature and is not shipped in ordinary release archives.

Source layout follows the same boundary: normal runtime modules remain directly under `src/`, while opt-in benchmark and release validation code lives under `src/maintenance/`.

Runtime state must remain bounded. Background execution limits both concurrently running Jobs and retained completed Job metadata/output; old completed Jobs are evicted together with their temporary log artifacts. New capabilities should prefer the same pattern: one canonical runtime path, bounded state, and opt-in compilation for functionality normal users do not need.

