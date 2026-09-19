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

> Status: pre-1.0. The local product workflow, macOS Seatbelt sandbox, approval system, structured Git mutations, multi-platform release packaging, Homebrew tap, and benchmark gates are implemented. Real ChatGPT Secure MCP Tunnel production acceptance evidence and 8 GiB Apple Silicon evidence are still release gates.

## Install

Download the archive for your platform from a GitHub Release and verify SHA256SUMS. Release archives include the matching official OpenAI tunnel-client runtime, so users do not install tunnel-client separately.

Supported release targets:

- macOS Intel: x86_64-apple-darwin, including Intel Macs on macOS 13
- macOS Apple Silicon: aarch64-apple-darwin for M1/M2/M3/M4
- Windows 10/11 x64: x86_64-pc-windows-msvc
- Windows 11 ARM64: aarch64-pc-windows-msvc
- Linux x86_64: x86_64-unknown-linux-gnu

The canonical repository is https://github.com/Chucklery/web-harness-rs.

Homebrew:

~~~bash
brew install Chucklery/tap/web-harness
~~~

Tap repository: https://github.com/Chucklery/homebrew-tap

Developers can also build from source:

~~~bash
cargo build --release
~~~

## First-time setup

setup is interactive on macOS, Windows, and Linux:

~~~bash
cd ~/code/my-project

web-harness setup
~~~

You will be prompted for:

- OpenAI tunnel_id
- OpenAI runtime API key

setup prints the current OpenAI Platform locations before prompting:

- https://platform.openai.com/
- tunnel_id: https://platform.openai.com/settings/organization/tunnels
- runtime API key: https://platform.openai.com/settings/organization/api-keys

The API key prompt disables terminal echo. On macOS/Linux, web-harness stores CONTROL_PLANE_TUNNEL_ID and CONTROL_PLANE_API_KEY in a managed block in ~/.zshrc (or ZDOTDIR/.zshrc). On Windows, it stores them in the current user's web-harness credentials.env file under APPDATA. The default flow launches the bundled official tunnel-client directly; no generated wrapper is required.

This is intentionally convenience-first: ~/.zshrc contains the API key in plaintext. web-harness changes the managed file to mode 0600 and makes a private backup before updates, but users who do not want plaintext shell configuration should keep using a custom wrapper or external secret mechanism instead.

Check the non-secret setup state with:

~~~bash
web-harness setup --show
~~~

For automation only, non-interactive setup is also available:

~~~bash
web-harness setup \
  --tunnel-id tunnel_0123456789abcdef0123456789abcdef \
  --api-key 'YOUR_RUNTIME_KEY'
~~~

Prefer interactive setup because command-line API keys may be retained in shell history or briefly visible to local process inspection.

## Daily use

In any repository:

~~~bash
cd ~/code/another-project
web-harness connect
~~~

connect uses the current directory as the workspace, starts the bundled OpenAI tunnel-client directly as an owned process, persists only non-secret runtime state, and returns immediately.

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

ChatGPT connector compatibility additionally exposes the lightweight control tools `runtime_status`, `work_on_project`, `tool_manifest`, and `call_runtime_tool`. They only describe or dispatch to the same bounded local runtime and do not embed another agent.

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
- interactive API-key entry with terminal echo disabled
- tunnel credentials stored in the user credential file only, not config.json or command arguments
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
web-harness serve --stdio --workspace .
~~~

Maintainer-only benchmark and release-gate commands are compiled only with the `release-tools` feature so normal release binaries stay small:

~~~bash
cargo run --release --features release-tools -- benchmark --workspace . --iterations 10000
cargo run --release --features release-tools -- release-gate --evidence benchmarks/example.json
~~~

GitHub Pages deployment is configured in .github/workflows/docs.yml.

## Contributing

See CONTRIBUTING.md and AGENTS.md.

## License

Apache-2.0. See LICENSE, NOTICE, and THIRD_PARTY_NOTICES.md.
