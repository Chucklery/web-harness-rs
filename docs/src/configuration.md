# Configuration

The bootstrap currently accepts the workspace on the CLI.

~~~bash
web-harness serve --stdio --workspace /path/to/project
~~~

A persistent TOML configuration file is planned but not yet implemented.

The architecture proposes future sections for:

- workspace roots
- security policy
- resource profiles
- search settings
- lightweight state

Secrets should not be stored in the ordinary project configuration.

