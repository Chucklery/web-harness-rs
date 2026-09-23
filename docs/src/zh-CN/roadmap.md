# 路线图

当前已实现：

- Rust Runtime
- MCP stdio
- 工作区安全边界
- 文件和命令能力
- Git 权限控制
- 有界 Job 观察、搜索续读与 Git 分页
- 显式审批的有界 Script 与一次性 stdin

以下项目尚未完成或尚未验收：

- 真实 ChatGPT Secure MCP Tunnel 端到端验收（当前只有本地 fixture/doctor 验收）。
- Apple Silicon 8 GiB 实机基准，以及 Tunnel + Host 峰值 RSS 证据。
- Linux Landlock 文件系统限制的上游文档可行性审查已完成；Linux enforcement 尚未实现，也未在 Linux 主机上测试。其权限能力依赖 ABI 版本，部分元数据操作无法限制，不能视为 Seatbelt 等价物。
- 原生 Windows 沙箱；当前未实现，继续使用逐次审批且不宣称有 OS 沙箱保护。
- 超出已记录 Intel Mac 快照的更广泛性能硬化。

UI 与 LSP 尚未实现，仍属可选项；只有真实使用数据证明必要时才纳入。
