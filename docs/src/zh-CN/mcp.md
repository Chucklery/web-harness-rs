# MCP 工具模型

web-harness 保持较小的工具集合，避免暴露过大的执行面。

主要能力包括：

- 工作区信息查询
- 文件读取
- 搜索
- AGENTS 指令发现
- 结构化修改
- 命令执行
- 后台任务管理
- Git 操作

## 搜索

使用系统中已安装的 ripgrep，这是唯一的运行时外部依赖。Homebrew 安装会自动带上 ripgrep；手动解压 Release 包的用户需要自行安装（`brew install ripgrep` / `apt install ripgrep`）。当 `PATH` 中找不到 `rg` 时，Search 会返回一条指明 ripgrep 及安装方式的依赖错误，其余工具不受影响。

## 命令执行

Exec 使用 argv 形式，不提供 shell 字符串模式。命令在启动前会做策略校验：对已知的 shell 与解释器（`sh`、`bash`、`python`、`ruby` 等）拒绝内联求值标志（`sh -c`、`bash -c`、`python -c`、`ruby -c`），并提示改用工作区内的脚本文件。这是策略过滤而非安全边界——`PATH` 中仍可能存在 `env` 这类包装器——真正的边界始终是下面的 sandbox。

macOS 下优先使用原生 Seatbelt。Sandbox profile 只允许写入工作区、TMPDIR、`/tmp`、`/private/tmp` 与 `/dev/null`。`/dev/null` 需要显式放行：shell、Git 以及大多数编译器与构建工具链都会无条件打开它，否则 `git status` 这类命令会直接以 `Operation not permitted` 失败。

被沙箱化的子进程会收到 `WEB_HARNESS_SANDBOX` 标记。若 web-harness 自身已经运行在 web-harness sandbox 内，该标记会阻止再次套用 Seatbelt——macOS 会以 `sandbox_apply: Operation not permitted` 拒绝嵌套 `sandbox-exec`，这正是此前通过 `exec` 运行 `cargo test` 失败的原因。该标记只抑制重复包装，外层 sandbox 依然生效。

## 审批

一次性票据绑定到精确请求，五分钟过期。票据只在授权成功时被消费；请求不匹配会报错并保留票据，因此用正确参数重试无需重新审批。Deny 会显式删除票据。

兼容控制接口不会引入第二个 Agent，它们只调用现有受限 Runtime。
