# ChatGPT Web 与安全 MCP Tunnel

生产连接方式设计为：

```text
ChatGPT Web -> Secure MCP Tunnel -> tunnel-client -> web-harness
```

本地 web-harness 使用 stdio MCP 作为通信方式，不需要暴露公网端口。

Tunnel 参数和认证流程应以 OpenAI 最新文档为准。
