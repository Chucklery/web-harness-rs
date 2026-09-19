# Third-Party Notices

This repository is designed with reference to OpenAI Codex and OpenAI Secure MCP Tunnel concepts.

At the current bootstrap stage, no source file is intentionally copied from the OpenAI Codex repository.

The structured patch syntax is behaviorally compatible with the public Codex-style patch format, but the parser and apply implementation in this repository are independently implemented and are not copied from Codex source.

If Codex-derived source is introduced later, record the upstream repository, upstream path, pinned commit, local destination, modification summary, and applicable license notice.

## OpenAI tunnel-client

Official web-harness release archives redistribute OpenAI's narrow `tunnel-client-runtime` artifact so users do not need to install a separate tunnel package.

- Upstream: https://github.com/openai/tunnel-client
- Pinned release: v0.0.14
- Flavor: `runtime` (run-only command surface)
- License: Apache License 2.0
- Redistributed payload: the runtime binary, renamed locally to `tunnel-client` for compatibility, plus upstream LICENSE/NOTICE, dependency license report, and SPDX manifest.
- Excluded payload: the full tunnel-client CLI, cloudflared, cloudflared manifest, admin/onboarding/Codex/profile-management command surface, and other files not required by the runtime bundle.
- Integrity: packaging pins the upstream `SHA256SUMS.txt` digest and verifies the selected runtime archive against that manifest before extraction.

The retained upstream legal and SPDX evidence is preserved inside `libexec/web-harness/` in each packaged release.

