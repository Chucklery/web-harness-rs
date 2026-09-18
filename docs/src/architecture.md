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

