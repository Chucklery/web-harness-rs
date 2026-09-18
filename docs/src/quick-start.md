# Quick Start

Validate a workspace:

~~~bash
cargo run -- workspace check .
~~~

Run environment checks:

~~~bash
cargo run -- doctor --workspace .
~~~

Run the local smoke test and microbenchmark:

~~~bash
cargo run -- self-test --workspace .
cargo run -- benchmark --workspace . --iterations 10000
~~~

Start the MCP host over stdio:

~~~bash
cargo run -- serve --stdio --workspace .
~~~

The current MCP tools are:

- workspace_info
- read_files
- search
- workspace_instructions
- patch
- exec
- job
- git
- permission

Arbitrary shell-string execution and Git remote mutation are intentionally not exposed. Native OS sandbox enforcement is not enabled yet, so exec requests require an approval ticket.

