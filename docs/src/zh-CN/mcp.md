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

工具结果在兼容文本投影之外同时提供有界的 `structuredContent`。客户端应优先使用结构化字段，把文本视为展示或兼容回退数据。

`tools/list` 包含保守的 MCP `ToolAnnotations` 交互提示，用于描述只读、破坏性和外部世界行为，帮助 Host 呈现确认界面；它们不是安全边界。真正的边界仍由 capability 检查、Host elicitation、工作区保护和 OS sandbox 执行。

`read_files` 保留字符串路径的兼容格式，也支持带 `start_line`、`end_line` 和 `expected_read_revision` 的对象路径。结果返回由 metadata 与文件首尾内容摘要组成的有界 `read_revision`；截断结果提供可直接续读的参数，文件版本变化时会以 Conflict 拒绝续读。Patch Update 的目标文件也有硬大小上限。

常见 protected path（`.env`、私钥、凭据等）会先返回一次性审批票据；Host 确认后，使用 `approval_id` 重试读取。

## 搜索

使用系统中已安装的 ripgrep，这是唯一的运行时外部依赖。Homebrew 安装会自动带上 ripgrep；手动解压 Release 包的用户需要自行安装（`brew install ripgrep` / `apt install ripgrep`）。当 `PATH` 中找不到 `rg` 时，Search 会返回一条指明 ripgrep 及安装方式的依赖错误，其余工具不受影响。

Search 还支持工作区相对 `scope`、literal 模式、有界 include/exclude glob，以及 `matches`、`files_with_matches`、`count` 输出模式。每次请求仍调用系统 ripgrep，不建立常驻索引。

单查询支持 `offset` 续读并返回 `next_offset`；续读时保持 query、scope、glob、mode 和结果上限不变。如果 ripgrep 原始输出触及硬上限，结果只标记 truncated，不提供伪造的 continuation。

如果显式 search scope 本身是 protected path，会要求与敏感文件读取相同的一次性审批；普通工作区范围搜索仍会过滤 protected path。

## 命令执行

Exec 默认使用 argv 形式；需要 pipes、重定向或短命令链时，也可使用有界的一次性 `script`。argv 命令在启动前会做策略校验：对已知的 shell 与解释器（`sh`、`bash`、`python`、`ruby` 等）拒绝内联求值标志（`sh -c`、`bash -c`、`python -c`、`ruby -c`），并提示改用显式 script 或工作区内的脚本文件。script 不与 `stdin` 同时接受，且始终需要一次性审批。这是策略过滤而非安全边界——`PATH` 中仍可能存在 `env` 这类包装器——真正的边界始终是下面的 sandbox。

通过 exec 直接运行的 Git 命令会在启动前分类，并复用结构化 Git 的审批 capability：`git push` 需要 `git.remote.write`，其他直接 Git 调用保守地要求 `git.local.write`。已批准的普通程序或包装器仍可能修改 sandbox 内的工作区，因此通用进程审批本身授予工作区范围内的执行权限。

## list_files

在不执行 shell 的情况下列出排序后的工作区相对路径、文件、目录和符号链接。默认只列 Git tracked 路径，也支持有界分页、`all` 数据源和简单 include glob。如果枚举本身触发硬扫描上限，结果会标记为 truncated，并且不会伪造可继续的 offset。

## job

Job 支持 poll、有限等待、list、cancel 以及 stdout/stderr 读取。output 可携带字节 cursor 做增量读取，并返回下一次 cursor；Host 会持续排空子进程管道，但每个流只保留有界的内存环形缓冲，过旧 cursor 会明确返回 truncated，不会因噪声输出增长无界状态。

Exec 可选接受一次性 UTF-8 `stdin`，上限 64 KiB。输入通过 Host 管理的临时文件提供给子进程，随后关闭；不提供 PTY 或交互式常驻会话，输入内容也会绑定到审批票据。

Exec 还提供明确的 `script` 模式：Unix 使用 `sh`/`bash`，Windows 使用 `powershell`/`pwsh`。脚本以有界的一次性 stdin 传入，并始终要求显式审批；不会建立常驻 shell。

macOS 下优先使用原生 Seatbelt。Sandbox profile 只允许写入工作区、TMPDIR、`/tmp`、`/private/tmp` 与 `/dev/null`。`/dev/null` 需要显式放行：shell、Git 以及大多数编译器与构建工具链都会无条件打开它，否则 `git status` 这类命令会直接以 `Operation not permitted` 失败。

网络策略按每次执行设置，默认是 `deny`。`outbound` 只增加 Seatbelt 的出站网络权限，并且必须使用绑定确切 argv、cwd、后台模式、capability 和网络策略的一次性审批。没有原生 sandbox backend 时会拒绝该升级，不提供 unsandboxed 网络模式。

## patch

Patch 支持有界的 Add、Update 和 Delete。路径始终受工作区边界保护；Add 会在边界检查后创建缺失的父目录，准备阶段失败会清理新建的空目录。可通过 `expected_read_revisions` 为 Update 提供读取版本 fence，文件变化时拒绝覆盖。

结构化 Git 的 `diff` 支持通过 `offset`/`limit` 返回有界的文件与 hunk 分页，并在结果中给出可复制的 `next_offset`；续读时保持其余 diff 参数不变。Git 还支持读取 revision 中的单个工作区文件、创建并切换分支，以及携带 `expected_head` 的 mutation；审批完成并在执行前会再次核验 HEAD，分支在审批期间变化时返回 Conflict。Commit 可携带 pathspec，避免把范围之外已有的暂存内容一并提交。

## 错误

可恢复的工具失败（包括参数错误、文件不存在、路径被 deny、不是普通文件、范围越界、编码错误、工作区冲突、权限拒绝、依赖缺失和命令执行失败）会以正常 `tools/call` result 返回，并设置 `isError: true`，同时在 `structuredContent.error` 提供机器可读错误。JSON-RPC 顶层 error 仅用于协议格式错误、未知 method，以及无法分派到工具的请求。

被沙箱化的子进程会收到 `WEB_HARNESS_SANDBOX` 标记。若 web-harness 自身已经运行在 web-harness sandbox 内，该标记会阻止再次套用 Seatbelt——macOS 会以 `sandbox_apply: Operation not permitted` 拒绝嵌套 `sandbox-exec`，这正是此前通过 `exec` 运行 `cargo test` 失败的原因。该标记只抑制重复包装，外层 sandbox 依然生效。

## 审批

一次性审批由 MCP Host 独占。当初始化后的客户端声明支持 elicitation 时，web-harness 会发送 `elicitation/create` 和有界确认表单，只有客户端返回接受后才执行。审批票据不会作为可调用 MCP 工具暴露，因此模型不能批准自己的请求，也不能通过 `call_runtime_tool` 绕过 Host。

不支持 elicitation 的客户端会收到 `approval_required`，需要等待 Host 侧审批机制；web-harness 不会静默执行。票据绑定精确请求，五分钟过期，并在一次成功授权后消费。

票据不匹配时不会被消费；拒绝会显式删除票据。

兼容控制接口不会引入第二个 Agent，它们只调用现有受限 Runtime。
