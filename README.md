# web-harness

web-harness is a lightweight native bridge that lets ChatGPT Web work with your local code, Git repository, and development commands without running a second local AI agent.

ChatGPT remains the agent and UI. web-harness is the local execution and permission boundary.

~~~text
ChatGPT Web
    |
    | OpenAI Secure MCP Tunnel
    v
web-harness
    |
    +-- files / search / AGENTS.md
    +-- structured patches
    +-- sandboxed commands / jobs
    +-- Git
    +-- approvals
    v
Your local repository
~~~

> Status: pre-1.0. The local product workflow, macOS Seatbelt sandbox, approval system, structured Git mutations, release packaging, and benchmark gates are implemented. A dedicated Homebrew tap, real ChatGPT Secure MCP Tunnel production acceptance evidence, and 8 GiB Apple Silicon evidence are still release gates.

## Install

Download the archive for your platform from a GitHub Release, verify SHA256SUMS, and put web-harness on PATH.

The canonical repository is https://github.com/Chucklery/web-harness-rs. A generated Homebrew formula is included with releases; a dedicated tap is not published yet.

Developers can also build from source:

~~~bash
cargo build --release
~~~

## First-time setup

Choose the current project and configure the external wrapper that invokes the current official OpenAI Secure MCP Tunnel flow:

~~~bash
cd ~/code/my-project

web-harness setup \
  --workspace . \
  --tunnel-wrapper /absolute/path/to/tunnel-wrapper
~~~

Do not put tokens, cookies, passwords, or API keys in wrapper arguments. Use the official login state or environment expected by the tunnel client.

The wrapper receives WEB_HARNESS_SERVER_BIN, WEB_HARNESS_SERVER_ARGS_JSON, and WEB_HARNESS_WORKSPACE.

## Daily use

In any repository:

~~~bash
cd ~/code/another-project
web-harness connect
~~~

connect uses the current directory as the workspace, starts the configured tunnel wrapper as an owned process group, persists only non-secret runtime state, and returns immediately.

Check or stop it with:

~~~bash
web-harness status
web-harness disconnect
~~~

Then use ChatGPT Web normally, for example:

- Read the project instructions and explain this repository.
- Fix this bug and run the tests.
- Review the current diff.
- Stage these files and create an atomic commit.
- Push this branch. This requires an explicit high-risk approval.

## Local capabilities

The MCP surface intentionally stays small:

- workspace_info
- read_files
- search
- workspace_instructions
- patch
- exec
- job
- git
- permission

Git supports structured status, diff, log, show, add, commit, switch, restore, and push. Mutations require one-time approvals; push is explicitly identified as a remote high-risk mutation. Arbitrary Git argv is not exposed.

## Security model

- canonical workspace/path guards
- bounded model-facing reads and outputs
- argv-based execution rather than arbitrary shell-string mode
- macOS deny-by-default Seatbelt execution when available
- network denied by the macOS sandbox profile
- workspace/TMP writes only
- child environment minimization
- output secret redaction
- one-time request-bound approvals
- structured Git mutations
- no persisted tunnel stdout/stderr during normal connect

Systems without a native sandbox backend fall back to explicit execution approvals.

See SECURITY.md and the documentation site for the full model and current limitations.

## Documentation and developer commands

The documentation site is built with mdBook:

~~~bash
mdbook serve docs
~~~

Advanced commands include:

~~~bash
web-harness doctor --workspace .
web-harness self-test --workspace .
web-harness tunnel doctor --workspace .
web-harness benchmark --workspace . --iterations 10000
web-harness release-gate --evidence benchmarks/example.json
web-harness serve --stdio --workspace .
~~~

GitHub Pages deployment is configured in .github/workflows/docs.yml.

## Architecture

The complete engineering blueprint lives in web-harness-final-architecture.md.

## Contributing

See CONTRIBUTING.md and AGENTS.md.

## License

Apache-2.0. See LICENSE, NOTICE, and THIRD_PARTY_NOTICES.md.
