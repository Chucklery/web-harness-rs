# ChatGPT Web 与安全 MCP Tunnel

生产连接方式设计为：

```text
ChatGPT Web -> Secure MCP Tunnel -> tunnel-client -> web-harness
```

本地 web-harness 使用 stdio MCP 作为通信方式，不需要暴露公网端口。

Tunnel 参数和认证流程应以 OpenAI 最新文档为准。

`web-harness tunnel doctor` 还会针对本地启动的 server 执行 initialize/tools-list，以及 workspace info、文件枚举、搜索和 Git status 的有界只读 `tools/call` roundtrip，并验证标准 tool-result envelope；不会修改配置的工作区。

真实 Tunnel 验收 wrapper 必须在成功退出时将且仅将一个 JSON 验收证据对象写入 stdout，诊断日志写入 stderr；仅退出码为 0 不再算验收通过。证据 schema v2 上限为 16 KiB，不允许自由文本或工作区内容。它必须包含 ChatGPT 客户端和 MCP 协议版本、十个全部通过的阶段（连接、项目发现、读取、搜索、修改、验证、后台任务、Git 检查、用户审批、断开清理）、有序的工具调用结果，以及失败恢复路径。证据记录使用真实 MCP 工具名：后台等待写作 `tool: "job", action: "wait"`，Git 检查写作 `tool: "git", action: "status"` 和 `tool: "git", action: "diff"`；不会接受虚构的 `job_wait`、`git_status`、`git_diff` 工具，也不会接受未暴露的 `permission` 工具。审批验证必须记录被 Host 阻止的调用及同一工具/操作随后成功重试。验收报告可保存为元数据，但不得加入凭据、原始工具参数/结果、源码片段或其他敏感工作区内容。详细字段和值见英文 [Tunnel 验收契约](../tunnel.md#acceptance-harness)。
