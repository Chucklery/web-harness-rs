# 安全模型

安全策略包括：

- 工作区边界限制
- 路径校验
- 命令执行控制
- 敏感操作确认

当前已为常见 `.env`、私钥和凭据文件提供统一 protected-path 分类：直接文件读取、列表/搜索过滤、Git revision 文件读取与 diff，以及可识别的 exec 路径参数都有相应保护。Git diff 会先扫描变更路径名，涉及受保护文件时审批前不返回正文；一次性脚本中可识别的 `git` 与 `push` token 还会额外要求 `git.remote.write`。该文本启发式不能完整审计任意脚本；脚本仍受显式审批和 OS sandbox 约束。真实生产 Secure MCP Tunnel 验收证据也仍待补齐。
