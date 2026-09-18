# web-harness

web-harness 是一个轻量的本地 Codex 风格执行 Host，目标是让 ChatGPT 网页端通过 MCP 安全地操作真实本地代码仓库，同时尽量降低常驻资源占用。

它不是第二个 Agent。ChatGPT 负责推理、规划与工具编排；web-harness 只负责本地执行能力。

当前状态：早期 bootstrap。已实现 Rust CLI、MCP stdio 骨架、workspace 校验、路径边界保护、workspace_info 和带总量限制的 read_files。Patch、exec、后台 jobs、Git、sandbox、approval 和 Tunnel 端到端仍在路线图中。

## 快速开始

~~~bash
cargo build
cargo run -- doctor --workspace .
cargo run -- workspace check .
cargo run -- serve --stdio --workspace .
~~~

## 文档站

项目使用 mdBook：

~~~bash
cargo install mdbook
mdbook serve docs
~~~

GitHub Pages 自动部署配置位于 .github/workflows/docs.yml。

## 核心原则

- ChatGPT 是上层 Agent。
- 本地 Host 只负责执行。
- 优先使用 OpenAI 官方 Secure MCP Tunnel。
- 本地 MCP 使用 stdio。
- 不依赖完整 codex-core。
- 所有资源都应有硬上限。
- 8 GB Mac 是核心目标，但必须通过真实 benchmark 后才宣称达到指标。

完整架构见 web-harness-final-architecture.md。

## 许可证

Apache-2.0。

