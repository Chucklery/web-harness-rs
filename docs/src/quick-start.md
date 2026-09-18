# Quick Start

Validate a workspace:

~~~bash
cargo run -- workspace check .
~~~

Run environment checks:

~~~bash
cargo run -- doctor --workspace .
~~~

Start the MCP host over stdio:

~~~bash
cargo run -- serve --stdio --workspace .
~~~

At this stage the exposed tools are intentionally minimal:

- workspace_info
- read_files

Do not expect patch, shell execution, Git mutation, jobs, or approvals yet.

