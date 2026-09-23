# 安全模型

安全策略包括：

- 工作区边界限制
- 路径校验
- 命令执行控制
- 敏感操作确认

当前已为常见 `.env`、私钥和凭据文件提供统一 protected-path 分类：直接文件读取、AGENTS 指令发现、列表/搜索过滤、Git revision 文件读取与 diff，以及可识别的 exec 路径参数都有相应保护。文件读取、指令发现、搜索范围、枚举和 exec 会先解析工作区内的符号链接目标，避免通过别名绕过检查。Git diff 会先扫描变更路径名，涉及受保护文件时审批前不返回正文；一次性脚本中可识别的 Git mutation token 还会按风险额外要求 `git.local.write` 或 `git.remote.write`。该文本启发式不能完整审计任意脚本；脚本仍受显式审批和 OS sandbox 约束。直接 Git 调用如果使用未知子命令，则要求 `git.remote.write`，因为该子命令可能是配置的 shell alias。

能够再启动其他程序的命令（包括 `env`、`xargs`、shell/解释器）即使处于 macOS sandbox 内，也必须获得一次性 `process.execute` 用户审批。审批绑定 argv、cwd、后台模式、stdin、网络策略和受保护读取标记。这样可避免已知的 opaque 启动形式静默绕过审批；其他任意开发工具仍以 OS sandbox 为最终边界。

Host elicitation 会显示 capability、简短操作摘要和审批原因，并提醒用户核对原始工具请求。由于 argv/stdin 可能含凭据，不会将其复制到本地审批展示记录。

macOS Seatbelt 测试覆盖默认禁止工作区外写入和本地网络连接、允许 workspace/TMPDIR 写入和系统工具读取，并验证显式开启 outbound 的子进程可建立出站 loopback 连接。

Linux Landlock 目前仅完成上游文档可行性审查，尚未实现，也未在 Linux 主机测试。其文件系统与网络权限依 ABI 版本而异，且无法限制 `chmod` 等部分元数据操作，因此不能声称与 Seatbelt 等价。非 macOS 执行仍依赖逐次显式审批。参见[上游 Landlock 用户态文档](https://docs.kernel.org/userspace-api/landlock.html)。真实生产 Secure MCP Tunnel 验收证据也仍待补齐。
