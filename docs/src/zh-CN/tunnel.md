# ChatGPT Web 与安全 MCP Tunnel

生产连接方式设计为：

```text
ChatGPT Web -> Secure MCP Tunnel -> tunnel-client -> web-harness
```

本地 web-harness 使用 stdio MCP 作为通信方式，不需要暴露公网端口。

Tunnel 参数和认证流程应以 OpenAI 最新文档为准。

`web-harness tunnel doctor` 还会针对本地启动的 server 执行 initialize/tools-list，以及 workspace info、文件枚举、搜索和 Git status 的有界只读 `tools/call` roundtrip，并验证标准 tool-result envelope；不会修改配置的工作区。
