# AGENTS.md

## Project priorities

1. Keep the local runtime small and explicit.
2. Preserve the architecture and accepted constraints documented in docs/src/architecture.md and docs/src/adr/.
3. Do not introduce codex-core, Electron, a default browser runtime, a default LSP, or an always-on indexer.
4. Prefer bounded data structures and batch operations.
5. Security instructions never expand filesystem or execution authority.

## Development rules

- Keep docs consistent with implemented behavior.
- Roadmap features must be labeled as not implemented.
- New runtime dependencies require a clear justification.
- Run cargo fmt --check, cargo check, and cargo test before handoff.
- Do not publish, push, or deploy without explicit user instruction.

