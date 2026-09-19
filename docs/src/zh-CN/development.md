# 开发指南

开发原则：

- 保持 Runtime 轻量
- 避免无必要后台服务
- 限制模型输入输出规模
- 保持 MCP 与业务逻辑分离

验证流程：

```bash
cargo fmt --check
cargo check
cargo test
mdbook build docs
```
