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

> 当前状态：pre-1.0。面向用户的连接生命周期、macOS Seatbelt、approval、结构化 Git 写操作、release 打包、Homebrew Tap 和 benchmark gate 已实现。真实 ChatGPT Secure MCP Tunnel 生产验收，以及 8 GiB Apple Silicon 实机证据仍是发布 Gate。

## 安装

从 GitHub Release 下载对应平台压缩包并使用 SHA256SUMS 校验。Release 已内置对应平台的 OpenAI 官方 tunnel-client 运行组件，用户不需要再单独安装 tunnel-client。

正式仓库地址为 https://github.com/Chucklery/web-harness-rs。

Homebrew：

~~~bash
brew install Chucklery/tap/web-harness
~~~

Tap 仓库：https://github.com/Chucklery/homebrew-tap

开发者也可以源码构建：

~~~bash
cargo build --release
~~~

## 第一次配置

普通 macOS/zsh 用户直接使用交互式 setup：

~~~bash
cd ~/code/my-project

web-harness setup
~~~

命令行会提示输入：

- OpenAI tunnel_id
- OpenAI runtime API key

setup 在输入前会直接提示获取地址：

- https://platform.openai.com/
- tunnel_id：https://platform.openai.com/settings/organization/tunnels
- runtime API key：https://platform.openai.com/settings/organization/api-keys

输入 API key 时终端不会回显。web-harness 会把 CONTROL_PLANE_TUNNEL_ID 和 CONTROL_PLANE_API_KEY 写入 ~/.zshrc（若设置了 ZDOTDIR，则写入 ZDOTDIR/.zshrc）中的受控 block，同时自动生成 tunnel wrapper；API key 不会写入 config.json 或 wrapper。

这是便利优先的方案：API key 会以明文存在于 ~/.zshrc。web-harness 会把该文件权限设为 0600，并在更新前创建私有备份；如果不接受明文 shell 配置，应继续使用自定义 wrapper 或其他 secret 管理方式。

查看不包含完整 key 的配置状态：

~~~bash
web-harness setup --show
~~~

自动化场景也支持非交互参数：

~~~bash
web-harness setup \
  --tunnel-id tunnel_0123456789abcdef0123456789abcdef \
  --api-key 'YOUR_RUNTIME_KEY'
~~~

普通用户优先使用交互模式，因为命令行参数中的 API key 可能进入 shell history，或短暂暴露给本机进程查看工具。

## 日常使用

进入任意仓库：

~~~bash
cd ~/code/another-project
web-harness connect
~~~

connect 默认把当前目录作为 workspace，通过已配置的 wrapper 启动随 web-harness 安装的官方 tunnel-client，并只持久化非敏感运行状态。

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
- API key 交互输入时关闭终端回显
- Tunnel 凭据只写入 web-harness 管理的 ~/.zshrc block，不写入 config.json 或自动生成的 wrapper
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

## License

Apache-2.0。
