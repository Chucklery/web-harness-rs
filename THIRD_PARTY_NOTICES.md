# Third-Party Notices

This repository is designed with reference to OpenAI Codex and OpenAI Secure MCP Tunnel concepts.

At the current bootstrap stage, no source file is intentionally copied from the OpenAI Codex repository.

The structured patch syntax is behaviorally compatible with the public Codex-style patch format, but the parser and apply implementation in this repository are independently implemented and are not copied from Codex source.

If Codex-derived source is introduced later, record the upstream repository, upstream path, pinned commit, local destination, modification summary, and applicable license notice.

## OpenAI tunnel-client

Official web-harness release archives redistribute the OpenAI tunnel-client release payload so users do not need to install a separate tunnel package.

- Upstream: https://github.com/openai/tunnel-client
- Pinned release: v0.0.14
- License: Apache License 2.0
- Redistributed payload: the official platform ZIP contents for the matching web-harness release target, including tunnel-client, cloudflared, upstream LICENSE/NOTICE, dependency license report, SPDX manifest, and cloudflared manifest.
- Integrity: scripts/fetch-tunnel-client.sh pins the upstream platform archive SHA256 before extraction.

The upstream LICENSE, NOTICE, dependency-license report, and SPDX files are preserved inside `libexec/web-harness/` in each packaged release.

