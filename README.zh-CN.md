# web-harness

web-harness 是一个轻量原生桥接器，让 ChatGPT 网页端直接操作本地代码、Git 仓库和开发命令，而不需要在本机再运行第二个 AI Agent。

ChatGPT 是 Agent 和 UI；web-harness 是本机执行层和权限边界。

~~~text
ChatGPT Web
    |
    | OpenAI Secure MCP Tunnel
    v
web-harness
    |
    +-- 文件 / 搜索 / AGENTS.md
    +-- 结构化 Patch
    +-- 沙箱命令 / 后台 Job
    +-- Git
    +-- Approval
    v
本地代码仓库
~~~

> 当前状态：pre-1.0。面向用户的连接生命周期、macOS Seatbelt、approval、结构化 Git 写操作、release 打包和 benchmark gate 已实现。正式公开仓库/Homebrew Tap、真实 ChatGPT Secure MCP Tunnel 生产验收，以及 8 GiB Apple Silicon 实机证据仍是发布 Gate。

## 安装

从 GitHub Release 下载对应平台压缩包，使用 SHA256SUMS 校验后，把 web-harness 放入 PATH。

Release 同时会生成 Homebrew Formula。等公开仓库地址固定后，再提供正式 Tap。

开发者也可以源码构建：

~~~bash
cargo build --release
~~~

## 第一次配置

进入一个项目，并配置用于调用当前 OpenAI 官方 Secure MCP Tunnel 流程的 wrapper：

~~~bash
cd ~/code/my-project

web-harness setup \
  --workspace . \
  --tunnel-wrapper /absolute/path/to/tunnel-wrapper
~~~

不要把 token、cookie、密码或 API key 写进 wrapper 参数。认证应使用官方登录态或 Tunnel 客户端需要的环境。

wrapper 会收到 WEB_HARNESS_SERVER_BIN、WEB_HARNESS_SERVER_ARGS_JSON 和 WEB_HARNESS_WORKSPACE。

## 日常使用

进入任意仓库：

~~~bash
cd ~/code/another-project
web-harness connect
~~~

connect 默认把当前目录作为 workspace，启动已配置的 Tunnel wrapper，并只持久化非敏感运行状态。

查看和断开：

~~~bash
web-harness status
web-harness disconnect
~~~

随后直接在 ChatGPT 网页端使用，例如：

- 读取项目规范并解释这个仓库。
- 修复这个 Bug，然后跑测试。
- Review 当前 diff。
- 把这些文件 stage 并做一个原子提交。
- Push 当前分支。该操作需要明确的高风险 approval。

## ChatGPT 可调用的本地能力

顶层 MCP 工具保持精简：

- workspace_info
- read_files
- search
- workspace_instructions
- patch
- exec
- job
- git
- permission

Git 支持结构化 status / diff / log / show / add / commit / switch / restore / push。所有 mutation 都需要一次性 approval；push 会明确标记为远程高风险操作。不开放任意 Git argv。

## 安全边界

- canonical workspace/path guard
- 所有模型输入输出有硬上限
- argv 执行，不提供任意 shell-string 模式
- macOS 默认使用 deny-by-default Seatbelt
- macOS 沙箱默认禁止网络
- 写权限限制在 workspace / TMP
- 子进程环境变量最小化
- 输出敏感信息脱敏
- approval 与具体请求绑定且一次性消费
- Git mutation 结构化
- 普通 connect 不把 Tunnel stdout/stderr 持久化到磁盘

没有原生 sandbox backend 的平台会回退为显式执行 approval。

## 开发者与诊断命令

~~~bash
web-harness doctor --workspace .
web-harness self-test --workspace .
web-harness tunnel doctor --workspace .
web-harness benchmark --workspace . --iterations 10000
web-harness release-gate --evidence benchmarks/example.json
web-harness serve --stdio --workspace .
~~~

文档站使用 mdBook，并通过 GitHub Pages 发布。

完整工程架构见 web-harness-final-architecture.md。

## License

Apache-2.0。
