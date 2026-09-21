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

`read_files` 保留字符串路径的兼容格式，也支持带 `start_line`、`end_line` 和 `expected_read_revision` 的对象路径。结果返回有界 `read_revision`；截断结果提供可直接续读的参数，文件版本变化时会以 Conflict 拒绝续读。

## 搜索

使用系统中已安装的 ripgrep，这是唯一的运行时外部依赖。Homebrew 安装会自动带上 ripgrep；手动解压 Release 包的用户需要自行安装（`brew install ripgrep` / `apt install ripgrep`）。当 `PATH` 中找不到 `rg` 时，Search 会返回一条指明 ripgrep 及安装方式的依赖错误，其余工具不受影响。

Search 还支持工作区相对 `scope`、literal 模式、有界 include/exclude glob，以及 `matches`、`files_with_matches`、`count` 输出模式。每次请求仍调用系统 ripgrep，不建立常驻索引。

## 命令执行

Exec 使用 argv 形式，不提供 shell 字符串模式。命令在启动前会做策略校验：对已知的 shell 与解释器（`sh`、`bash`、`python`、`ruby` 等）拒绝内联求值标志（`sh -c`、`bash -c`、`python -c`、`ruby -c`），并提示改用工作区内的脚本文件。这是策略过滤而非安全边界——`PATH` 中仍可能存在 `env` 这类包装器——真正的边界始终是下面的 sandbox。

通过 exec 直接运行的 Git 命令会在启动前分类，并复用结构化 Git 的审批 capability：`git push` 需要 `git.remote.write`，其他直接 Git 调用保守地要求 `git.local.write`。已批准的普通程序或包装器仍可能修改 sandbox 内的工作区，因此通用进程审批本身授予工作区范围内的执行权限。

## list_files

在不执行 shell 的情况下列出排序后的工作区相对路径、文件、目录和符号链接。默认只列 Git tracked 路径，也支持有界分页、`all` 数据源和简单 include glob。如果枚举本身触发硬扫描上限，结果会标记为 truncated，并且不会伪造可继续的 offset。

## job

Job 支持 poll、有限等待、list、cancel 以及 stdout/stderr 读取。output 可携带字节 cursor 做增量读取，并返回下一次 cursor；输出始终写入有界临时文件，Host 退出时终止所属进程组。

Exec 可选接受一次性 UTF-8 `stdin`，上限 64 KiB。输入通过 Host 管理的临时文件提供给子进程，随后关闭；不提供 PTY 或交互式常驻会话，输入内容也会绑定到审批票据。

Exec 还提供明确的 `script` 模式：Unix 使用 `sh`/`bash`，Windows 使用 `powershell`/`pwsh`。脚本以有界的一次性 stdin 传入，并始终要求显式审批；不会建立常驻 shell。

macOS 下优先使用原生 Seatbelt。Sandbox profile 只允许写入工作区、TMPDIR、`/tmp`、`/private/tmp` 与 `/dev/null`。`/dev/null` 需要显式放行：shell、Git 以及大多数编译器与构建工具链都会无条件打开它，否则 `git status` 这类命令会直接以 `Operation not permitted` 失败。

网络策略按每次执行设置，默认是 `deny`。`outbound` 只增加 Seatbelt 的出站网络权限，并且必须使用绑定确切 argv、cwd、后台模式、capability 和网络策略的一次性审批。没有原生 sandbox backend 时会拒绝该升级，不提供 unsandboxed 网络模式。

## patch

Patch 支持有界的 Add、Update 和 Delete。路径始终受工作区边界保护；Add 会在边界检查后创建缺失的父目录，准备阶段失败会清理新建的空目录。可通过 `expected_read_revisions` 为 Update 提供读取版本 fence，文件变化时拒绝覆盖。

结构化 Git 的 `diff` 支持通过 `offset`/`limit` 返回有界的文件与 hunk 分页，并在结果中给出可复制的 `next_offset`；续读时保持其余 diff 参数不变。Git 还支持读取 revision 中的单个工作区文件、创建并切换分支，以及携带 `expected_head` 的 mutation；审批完成并在执行前会再次核验 HEAD，分支在审批期间变化时返回 Conflict。Commit 可携带 pathspec，避免把范围之外已有的暂存内容一并提交。

被沙箱化的子进程会收到 `WEB_HARNESS_SANDBOX` 标记。若 web-harness 自身已经运行在 web-harness sandbox 内，该标记会阻止再次套用 Seatbelt——macOS 会以 `sandbox_apply: Operation not permitted` 拒绝嵌套 `sandbox-exec`，这正是此前通过 `exec` 运行 `cargo test` 失败的原因。该标记只抑制重复包装，外层 sandbox 依然生效。

## 审批

一次性票据绑定到精确请求，五分钟过期。票据只在授权成功时被消费；请求不匹配会报错并保留票据，因此用正确参数重试无需重新审批。Deny 会显式删除票据。

`permission` 工具通过 MCP `ToolAnnotations` 声明为 destructive，因此 ChatGPT 会在审批调用到达 web-harness 前要求用户确认。该注解构成用户交互边界；进程内权限引擎继续负责服务端的精确请求绑定和一次性消费。

`permission` 只能直接调用。adaptive `call_runtime_tool` 网关会拒绝转发它，防止利用网关的通用注解绕过 Host 确认。

兼容控制接口不会引入第二个 Agent，它们只调用现有受限 Runtime。
