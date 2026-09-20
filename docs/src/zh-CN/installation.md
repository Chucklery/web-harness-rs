# 安装

## 环境要求

- macOS / Linux
- Rust 工具链（仅源码构建需要）
- ripgrep（`search` 工具需要；Homebrew 会自动安装）

## Homebrew

```bash
brew install Chucklery/tap/web-harness
```

formula 已声明 `depends_on "ripgrep"`，安装后 `search` 即可用。

## GitHub Release

从 Release 页面下载对应平台归档并校验 SHA256SUMS。Release 包内已包含配套的官方 OpenAI tunnel-client，无需单独安装。

```bash
tar -xzf web-harness-VERSION-TARGET.tar.gz
./web-harness version
```

除 tunnel-client 外，`search` 依赖系统 ripgrep，需要自行安装：

```bash
# macOS
brew install ripgrep

# Debian/Ubuntu
sudo apt install ripgrep

# Fedora
sudo dnf install ripgrep
```

缺少 ripgrep 时只有 `search` 会返回明确的依赖错误，其他工具均正常可用。

## 构建

```bash
cargo build --release
```
