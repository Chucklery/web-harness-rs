# web-harness

web-harness 是一个轻量的本地 Codex 风格执行 Host，目标是让 ChatGPT 网页端通过 MCP 安全地操作真实本地代码仓库，同时尽量降低常驻资源占用。

它不是第二个 Agent。ChatGPT 负责推理、规划与工具编排；web-harness 只负责本地执行能力。

当前状态：pre-1.0 持续开发。已实现 Rust CLI、MCP stdio Host、workspace/path guard、AGENTS 作用域发现、受限读取/搜索、结构化 Patch、前台/后台进程执行、JobManager、结构化只读 Git Gateway，以及与具体执行请求绑定的一次性 approval。原生 OS sandbox 与真实 ChatGPT Secure MCP Tunnel 端到端验收仍未完成。

## 快速开始

~~~bash
cargo build
cargo run -- doctor --workspace .
cargo run -- workspace check .
cargo run -- self-test --workspace .
cargo run -- benchmark --workspace . --iterations 10000
cargo run -- tunnel doctor --workspace .
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

