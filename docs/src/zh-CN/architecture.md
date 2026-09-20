# 架构设计

```
ChatGPT Web
    |
    | MCP
    |
web-harness Runtime
    |
本地工作区 / Git / Shell
```

web-harness 保持推理层与执行层分离。

## Tool Runtime 边界

MCP 传输层开始与本地工具执行解耦。MCP 层只负责协议解析、响应封装和传输层错误映射；Runtime 工具负责受边界约束的本地工作区操作。

Phase 1 新增 `RuntimeTool` 接口与 `RuntimeRegistry`。目前 `read_files` 与 Search 已迁移到统一 Runtime 注册与调用入口，并同时覆盖直接 MCP 调用与 adaptive-runtime 兼容网关。Exec、Job、Git、Patch 与 Permission 仍暂时保留现有 MCP dispatch，后续阶段再逐步迁移。

该抽象不会引入第二进程、模型循环、守护进程、数据库或新的运行时依赖。
