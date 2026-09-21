# 安全模型

安全策略包括：

- 工作区边界限制
- 路径校验
- 命令执行控制
- 敏感操作确认

当前未实现针对 `.env`、私钥和凭据文件的统一 protected-path 分类及逐次读取授权；实际边界仍依赖配置的 deny paths、sandbox 和输出脱敏。真实生产 Secure MCP Tunnel 验收证据也仍待补齐。
