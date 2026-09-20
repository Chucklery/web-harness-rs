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

`RuntimeTool` 接口与 `RuntimeRegistry` 现在统一调度全部 9 个直接工具：Workspace 信息/指令、文件读取、Search、Patch、Exec、Job 控制、Git 与审批票据管理。直接 MCP 调用和 adaptive-runtime 兼容网关共用同一 Registry 调用路径。

该抽象不会引入第二进程、模型循环、守护进程、数据库或新的运行时依赖。

### ExecutionContext 与 Capability

Runtime 调用统一接收 `ExecutionContext`，其中包含工作区边界、Sandbox 后端、权限引擎、资源限制、非敏感平台信息以及 Job manager。安全与资源策略不再由每个 MCP handler 重复处理。

权限模型使用显式 Capability，例如 `workspace.read`、`workspace.write`、`process.execute`、`job.control`、`git.read`、`git.local.write` 与 `git.remote.write`。一次性审批票据同时绑定 Capability 与精确命令参数，不能把低权限审批复用于更高权限操作。

Git 使用独立 Runtime 权限策略，不经过通用 exec sandbox。只读操作（`status`、`diff`、`log`、`show`）使用 `git.read`；本地修改（`add`、`commit`、`switch`、`restore`）使用 `git.local.write`；`push` 使用 `git.remote.write`。本地写与远端写都绑定各自精确命令并要求一次性审批。

Runtime Status 与 Tool Manifest 分别从 `ExecutionContext` 和 `RuntimeRegistry` 动态生成，不再在 MCP 层维护重复常量。MCP 模块只保留 JSON-RPC/MCP 封装、4 个 adaptive 兼容控制调用以及传输层错误码映射。
