# Web Harness Lite：ChatGPT Web ↔ Local Codex-Style Runtime 最终开发方案

> 状态：Final Architecture / Implementation Blueprint  
> 目标平台：macOS（优先，Apple Silicon / Intel），后续扩展 Linux / Windows  
> 目标机器：低端 8 GB RAM Mac 也能长期常驻、流畅使用  
> 核心目标：让 **ChatGPT 网页端直接、安全、低延迟地调用本机开发环境**，获得尽量接近本地 Codex 的代码探索、编辑、执行、测试、Git、审批和连续任务体验，同时避免把完整 Codex / WebCodex 的重型运行时搬到本机。  
> 文档基线：2026-09-18

---

## 0. 一句话结论

最终架构采用：

```text
ChatGPT Web
    │
    │ MCP / OpenAI Secure MCP Tunnel
    ▼
OpenAI tunnel-client
    │
    │ stdio MCP
    ▼
web-harness-host
    │  Rust，单本地执行 Host
    │
    ├── workspace / AGENTS
    ├── batch read / search
    ├── Codex-style apply_patch
    ├── exec / PTY-lite / background jobs
    ├── git
    ├── sandbox
    ├── approval
    ├── compact session state
    └── optional external MCP gateway
         │
         ▼
      Local Repository
```

原则：

1. **ChatGPT 负责模型、推理、对话与 agent loop。**
2. **本机 Host 只负责本地执行能力。**
3. **OpenAI 官方 `tunnel-client` 负责 ChatGPT ↔ 本机网络传输。**
4. **复用 Codex 的设计与必要代码，但绝不依赖完整 `codex-core`。**
5. **无 Electron、无 Node 常驻、无 Browser 常驻、无 LSP 常驻、无索引服务、无 Server→Runner 二跳。**
6. **默认仅两个常驻原生进程：`tunnel-client` + `web-harness-host`。**
7. **8 GB Mac 是一等公民，不是事后兼容对象。**

---

# 1. 项目定位

项目暂称：

```text
web-harness
```

其本质不是：

- WebCodex 的缩小版；
- 一个远程 Shell；
- 一个 `read_file/write_file` MCP Demo；
- 一个套壳 Codex CLI；
- 一个 Agent 调另一个 Agent 的桥；
- 一个必须常驻浏览器 UI 的 Desktop App。

它应该被定义为：

> **Local Codex-Style Execution Host for ChatGPT Web**

更准确地说：

```text
ChatGPT = Brain / Conversation / Planning / Tool Orchestration

web-harness-host = Local Hands / Sandbox / Workspace / Process / Git

OpenAI Secure MCP Tunnel = Secure Transport
```

这样可以最大程度避免功能重复。

---

# 2. 核心目标

## 2.1 必须实现

ChatGPT 网页端连接后，应能完成完整开发循环：

```text
理解项目
  ↓
读取 AGENTS.md / 项目约束
  ↓
搜索代码
  ↓
批量读取文件
  ↓
修改一个或多个文件
  ↓
查看 diff
  ↓
运行 lint / test / build
  ↓
读取错误
  ↓
继续修改
  ↓
启动后台任务
  ↓
检查后台任务状态
  ↓
Git status / diff / log
  ↓
完成任务
```

体验目标不是“拥有某几个工具”，而是：

> **让 ChatGPT Web 具有足够完整、低摩擦的本地执行面，从而形成类似 Codex CLI 的 coding-agent loop。**

---

## 2.2 强制性能目标

8 GB Mac 基线：

| 指标 | 工程目标 |
|---|---:|
| Host idle CPU | ≈ 0% |
| Host idle RSS | `< 60–80 MB` |
| Tunnel + Host idle RSS | `< 120–150 MB` |
| UI 关闭时 Chromium/WebKit | 0 |
| Node/Bun 常驻 | 0 |
| 默认 LSP | 0 |
| 默认 file watcher | 0 |
| 默认全文索引服务 | 0 |
| 默认 embedding/vector DB | 0 |
| 空闲子进程 | 0 |
| 本地 tool dispatch p50 | `< 5 ms` |
| 本地 tool dispatch p95 | `< 20 ms` |
| 单 Job 内存日志缓存 | `<= 256–512 KiB` |
| Host 冷启动目标 | `< 500 ms`（机器相关） |
| 单次普通文件读取额外 copy | 尽量 ≤ 1 次 |
| MCP schema 顶层 tools | 推荐 `8–10` 个 |

说明：

这些是 **验收 Gate**，不是当前实现已有的实测数字。开发完成后必须在真实 8 GB Intel Mac 与 Apple Silicon Mac 上 benchmark。

---

# 3. 明确非目标

V1 不做：

- 多用户 Server；
- 多机器 Runner；
- 企业 RBAC；
- Cloud Agent；
- Durable Agent Graph；
- Conversation 数据库；
- 长期模型记忆；
- 模型客户端；
- 内置 OpenAI Responses 调用；
- 本地再次运行 Codex Agent；
- Electron；
- Tauri 主窗口常驻；
- Web Browser Agent；
- Computer Use；
- 多 Agent 编排；
- 常驻 LSP；
- 全仓 AST 索引；
- Embedding；
- Vector DB；
- Git hosting 平台管理；
- Docker orchestration；
- Remote Runner。

原因很简单：

这些不是实现“ChatGPT 网页端拥有本地 Codex 风格开发能力”的最短关键路径。

---

# 4. 为什么不直接复用整个 Codex

OpenAI Codex 当前 Rust workspace 已经非常大，包含例如：

```text
core
app-server
app-server-daemon
app-server-client
agent-graph-store
cloud-tasks
connectors
history
memories
model-provider
responses-api-proxy
realtime-webrtc
tui
file-watcher
network-proxy
worktree
...
```

如果直接：

```text
web-harness-host
  └── codex-core
```

会出现三个问题。

## 4.1 依赖树失控

即使最终运行时没有启动某些组件：

- 编译成本上升；
- binary 体积上升；
- 安全审计范围扩大；
- API 变化耦合；
- feature 管理复杂；
- 上游 Codex 重构容易破坏 Host。

## 4.2 上层 Agent 重复

本项目的上层 Agent 已经是 ChatGPT Web。

若再把 Codex Agent Runtime 嵌入：

```text
ChatGPT Agent
    ↓
Codex Agent
    ↓
Local Tools
```

会产生：

- 双模型；
- 双 context；
- 双 planning；
- 双 tool orchestration；
- 更高延迟；
- 更高 token 成本；
- 更难调试。

## 4.3 与“轻量”目标相冲突

真正应该复用的是：

> Codex 已验证过的本地执行、安全、patch、路径与输出处理思想。

而不是：

> Codex 的完整产品 runtime。

---

# 5. Codex 复用策略

采用三档策略。

## 5.1 A 类：可以直接复用的小型、独立能力

候选：

```text
absolute-path
path-uri
output-truncation
stream-parser
部分 git/path utils
```

判断标准：

- 依赖少；
- API 稳定；
- 无模型层；
- 无网络层；
- 无大 runtime；
- 不引入 Tokio full / Web server / telemetry 全家桶；
- 编译后确实比自己实现更省维护成本。

---

## 5.2 B 类：复用源码/算法，但不直接依赖上游 crate

重点：

```text
apply_patch parser / hunk model
patch validation
AGENTS discovery semantics
sandbox policy semantics
approval semantics
command safety classification ideas
output truncation behavior
```

例如 Codex 的 `codex-apply-patch` 很成熟，但当前 crate 已经存在对其它 Codex execution 层的依赖。

因此推荐：

```text
vendor/
  codex-derived/
    patch/
```

或者：

```text
crates/
  patch/
```

只抽取真正需要的：

```text
parser.rs
hunk.rs
apply.rs
errors.rs
```

并明确标注来源与修改。

---

## 5.3 C 类：只参考设计，不复用代码

包括：

```text
codex-core
codex-sandboxing 整 crate
codex-execpolicy 整 crate
codex-file-search
app-server
thread-store
agent graph
history
cloud tasks
connectors
TUI
```

这些组件直接依赖会显著扩大 runtime。

---

# 6. 开源许可证策略

建议项目本身采用：

```text
Apache-2.0
```

原因：

- Codex 为 Apache-2.0；
- OpenAI `tunnel-client` 为 Apache-2.0；
- 与必要的源码复用最简单；
- 对未来开源分发、商业使用、贡献都比较友好。

仓库必须包含：

```text
LICENSE
NOTICE
THIRD_PARTY_NOTICES.md
```

对于直接复制或修改的 Codex 文件：

1. 保留原 copyright / license notice；
2. 标记文件已修改；
3. 在 `THIRD_PARTY_NOTICES.md` 记录：
   - upstream repository；
   - upstream path；
   - upstream commit SHA；
   - 本项目对应文件；
   - 修改说明；
4. 定期做 upstream diff audit。

不要长期从 `main` 隐式同步。

建议 pin：

```text
CODEX_UPSTREAM_REVISION
```

例如：

```text
third_party/codex-revision.txt
```

---

# 7. 网络与连接架构

OpenAI Secure MCP Tunnel 官方支持：

- MCP Server 位于 developer machine / private network；
- `tunnel-client` 通过出站 HTTPS 连接 OpenAI；
- 本地 MCP 可通过 `stdio` 或 HTTP；
- ChatGPT developer-mode app 可以选择 Tunnel；
- 本地 MCP 无需公网监听；
- `tunnel-client` 负责 long-poll work 与 response posting。

因此 V1 使用官方最短链：

```text
ChatGPT Web
      │
      ▼
OpenAI hosted tunnel endpoint
      │
      ▼
tunnel-client
      │
      │ stdio
      ▼
web-harness-host
```

不添加：

```text
localhost HTTP MCP
reverse proxy
Cloudflare
Nginx
WebCodex Server
Runner network bridge
QUIC
WebSocket
```

---

# 8. 为什么选择 stdio 而不是 localhost HTTP

对于单用户本机：

## stdio 优点

- 少一个 listening socket；
- 少一层 HTTP parse；
- 少一套 auth；
- 少一套 CORS/CSRF/host binding 问题；
- 生命周期天然由 `tunnel-client` 管理；
- Host 崩溃更容易被检测；
- 没有额外端口占用；
- 本机攻击面更小。

数据路径：

```text
tunnel-client stdin/stdout
      ↕
web-harness-host MCP JSON-RPC
```

日志必须走：

```text
stderr
```

绝不能污染 stdout MCP stream。

---

# 9. 进程模型

默认只有：

```text
PID A: tunnel-client
PID B: web-harness-host
```

当执行任务时才动态创建：

```text
rg
git
npm
pnpm
cargo
pytest
...
```

任务结束立即回收。

## 9.1 不需要自研 Supervisor

优先让官方 tunnel-client 使用：

```text
--mcp-command
```

启动：

```text
web-harness-host serve --stdio
```

这样：

```text
tunnel-client
   └── child: web-harness-host
```

生命周期更简单。

---

# 10. Host 技术选型

## 10.1 Rust

选择 Rust，不选 Node/Bun 的原因：

- 常驻内存更可控；
- 无 GC pause；
- 无 JS runtime；
- 单 binary；
- 与 Codex 核心实现语言一致；
- 可以有选择地复用 Codex Rust 代码；
- 进程、PTY、路径、sandbox 操作更自然；
- macOS Seatbelt 适配更直接。

## 10.2 Tokio

禁止：

```toml
tokio = { version = "1", features = ["full"] }
```

建议：

```toml
tokio = {
  version = "1",
  default-features = false,
  features = [
    "rt",
    "macros",
    "process",
    "io-util",
    "fs",
    "sync",
    "time",
    "signal"
  ]
}
```

初期 benchmark：

```text
current_thread runtime
vs
multi_thread(worker_threads = 2)
```

MCP 自身不是高并发 HTTP Server，因此不需要十几个 worker。

---

# 11. 推荐仓库结构

```text
web-harness/
├── Cargo.toml
├── Cargo.lock
├── LICENSE
├── NOTICE
├── THIRD_PARTY_NOTICES.md
├── README.md
├── README.zh-CN.md
├── AGENTS.md
│
├── crates/
│   ├── protocol/
│   │   ├── src/
│   │   │   ├── mcp.rs
│   │   │   ├── tool.rs
│   │   │   ├── error.rs
│   │   │   └── schema.rs
│   │
│   ├── workspace/
│   │   ├── src/
│   │   │   ├── root.rs
│   │   │   ├── path_guard.rs
│   │   │   ├── agents.rs
│   │   │   └── metadata.rs
│   │
│   ├── fs/
│   │   ├── src/
│   │   │   ├── read.rs
│   │   │   ├── list.rs
│   │   │   └── limits.rs
│   │
│   ├── search/
│   │   ├── src/
│   │   │   ├── rg.rs
│   │   │   ├── files.rs
│   │   │   └── result.rs
│   │
│   ├── patch/
│   │   ├── src/
│   │   │   ├── parser.rs
│   │   │   ├── hunk.rs
│   │   │   ├── apply.rs
│   │   │   ├── validate.rs
│   │   │   └── errors.rs
│   │
│   ├── exec/
│   │   ├── src/
│   │   │   ├── spawn.rs
│   │   │   ├── job.rs
│   │   │   ├── output.rs
│   │   │   ├── process_tree.rs
│   │   │   └── limit.rs
│   │
│   ├── git/
│   │   ├── src/
│   │   │   ├── status.rs
│   │   │   ├── diff.rs
│   │   │   ├── log.rs
│   │   │   └── command.rs
│   │
│   ├── sandbox/
│   │   ├── src/
│   │   │   ├── policy.rs
│   │   │   ├── permissions.rs
│   │   │   └── macos/
│   │   │       ├── seatbelt.rs
│   │   │       └── profile.rs
│   │
│   ├── approval/
│   │   ├── src/
│   │   │   ├── decision.rs
│   │   │   ├── ticket.rs
│   │   │   ├── rules.rs
│   │   │   └── hash.rs
│   │
│   ├── jobs/
│   │   ├── src/
│   │   │   ├── registry.rs
│   │   │   ├── artifact.rs
│   │   │   └── cleanup.rs
│   │
│   ├── state/
│   │   ├── src/
│   │   │   ├── session.rs
│   │   │   └── persistence.rs
│   │
│   └── host/
│       ├── src/
│       │   ├── main.rs
│       │   ├── server.rs
│       │   ├── dispatch.rs
│       │   └── config.rs
│
├── apps/
│   └── ui/                  # P2，可选
│
├── benches/
│   ├── dispatch.rs
│   ├── read.rs
│   ├── search.rs
│   └── memory/
│
├── tests/
│   ├── e2e/
│   ├── sandbox/
│   ├── patch/
│   └── fixtures/
│
└── docs/
    ├── ARCHITECTURE.md
    ├── SECURITY.md
    ├── MCP.md
    ├── PERFORMANCE.md
    ├── CODEX_REUSE.md
    └── DEVELOPMENT.md
```

---

# 12. 模块边界规则

这是防止未来再次膨胀的关键。

依赖层：

```text
protocol
   ↑
workspace fs search patch exec git sandbox approval state
   ↑
host
```

禁止：

```text
workspace -> host
patch -> host
exec -> MCP server
sandbox -> UI
git -> state
```

基础模块不能依赖组合层。

建议 CI 加：

```text
workspace-boundaries.toml
```

或自定义脚本检查 crate dependency graph。

---

# 13. MCP Tool Surface

不要暴露 40~80 个 tools。

推荐初版 **8 个顶层工具**：

```text
workspace
read
search
patch
exec
job
git
permission
```

可选第 9 个：

```text
extension
```

用于后续调用外部 MCP。

---

# 14. Tool 1：workspace

职责：

- 获取项目信息；
- 获取适用 AGENTS；
- 获取 Git metadata；
- 获取能力；
- 切换允许的注册项目（后期）。

请求：

```json
{
  "action": "info"
}
```

返回示意：

```json
{
  "root": "/Users/me/project",
  "git": {
    "is_repo": true,
    "branch": "main"
  },
  "instructions": {
    "files": [
      "AGENTS.md",
      "src/AGENTS.md"
    ]
  },
  "capabilities": {
    "patch": true,
    "exec": true,
    "sandbox": "workspace-write"
  }
}
```

注意：

默认不要返回整个 AGENTS 内容。

可：

```json
{
  "action": "instructions",
  "paths": ["src/app.ts"]
}
```

只解析与目标路径相关的 instruction chain。

---

# 15. AGENTS.md 语义

参考 Codex 行为：

```text
AGENTS.md
AGENTS.override.md
```

作用域规则：

```text
repo/AGENTS.md
    ↓ applies repo/**

repo/src/AGENTS.md
    ↓ applies repo/src/**

repo/src/foo/AGENTS.override.md
    ↓ override deeper scope
```

实现方式：

1. Workspace open 时只扫描祖先链；
2. 不递归扫描整个仓库；
3. 当第一次访问一个目录时 lazy discover；
4. 缓存：
   ```text
   directory -> effective_instruction_chain
   ```
5. 缓存有最大数量；
6. 文件 mtime 变化时只失效对应目录链；
7. 默认不开全局 watcher。

这样可以保留 Codex 项目约束体验，但不常驻扫描仓库。

---

# 16. Tool 2：read

必须支持批量读取，禁止 ChatGPT 为三份文件产生三次 Tunnel RTT。

请求：

```json
{
  "paths": [
    "src/main.ts",
    "src/client.ts",
    "package.json"
  ],
  "range": {
    "start_line": 1,
    "end_line": 300
  },
  "max_bytes_per_file": 131072,
  "max_total_bytes": 524288
}
```

返回：

```json
{
  "files": [
    {
      "path": "src/main.ts",
      "content": "...",
      "truncated": false
    }
  ],
  "complete": true
}
```

限制：

```text
max files/call
max bytes/file
max bytes/call
binary detection
UTF-8 handling
symlink handling
```

默认：

```text
不要读取：
.git/objects
node_modules
target
dist
大二进制
```

除非显式请求且在权限范围。

---

# 17. 路径安全模型

任何 File/Patch/Exec/Git 请求都必须先进入统一：

```text
PathGuard
```

流程：

```text
user/model path
    ↓
normalize
    ↓
join workspace
    ↓
canonicalize nearest existing ancestor
    ↓
resolve symlink
    ↓
check allowed root
    ↓
operation-specific permission
```

核心不变量：

```text
任何一个内部 tool 都不能拥有自己的“特殊路径绕过逻辑”。
```

尤其要避免：

```text
shell 有 write whitelist
apply_patch 却只检查 cwd
```

所有写能力：

```text
write
patch
rename
delete
shell-generated write
artifact export
```

必须共享统一 write roots policy。

---

# 18. Tool 3：search

V1 不创建索引。

后端：

```text
rg
rg --files
git ls-files
```

请求：

```json
{
  "queries": [
    {
      "pattern": "createClient",
      "glob": ["src/**/*.ts"]
    },
    {
      "pattern": "API_BASE_URL"
    }
  ],
  "max_results": 100,
  "context_lines": 2
}
```

优化：

```text
batch multiple queries
```

而不是：

```text
search call × N
```

策略：

```text
tracked repo:
  filenames -> git ls-files

generic repo:
  filenames -> rg --files

content:
  rg --json
```

禁止默认：

- ripgrep daemon；
- file watcher；
- Tantivy；
- SQLite FTS；
- vector search；
- embeddings。

以后只有 benchmark 证明大仓库确实需要时再加 optional index feature。

---

# 19. Tool 4：patch

Patch 是接近 Codex 体验的关键。

建议支持 Codex 风格：

```text
*** Begin Patch
*** Update File: src/main.ts
@@
-old
+new
*** End Patch
```

操作：

```text
Add File
Update File
Delete File
Move/Rename
```

执行流程：

```text
parse
 ↓
collect affected paths
 ↓
resolve AGENTS instructions
 ↓
PathGuard
 ↓
permission policy
 ↓
approval if required
 ↓
read original
 ↓
validate context / hunks
 ↓
apply in memory
 ↓
atomic write
 ↓
return changed paths + compact diff
```

重要：

> Patch 不能因为是“内部结构化工具”就绕过 sandbox。

统一调用：

```rust
permission_engine.authorize(FileOperation::Write(...))
```

---

# 20. Patch 原子性

单文件：

```text
write temp
fsync if configured
rename
```

多文件：

V1 推荐：

```text
validate all first
then apply sequentially
```

若中间失败：

```text
return partial failure + changed set
```

P1 再考虑 transaction-like rollback。

不要为了模拟数据库事务而引入复杂 snapshot 系统。

Git 本身就是开发场景最自然的恢复机制。

---

# 21. Tool 5：exec

目标不是简单：

```text
system("npm test")
```

而是结构化 process execution。

请求：

```json
{
  "argv": ["pnpm", "test"],
  "cwd": ".",
  "timeout_ms": 120000,
  "background": false,
  "env": {},
  "permissions": {
    "network": false
  }
}
```

禁止 V1 默认接受：

```json
{
  "command": "任意大字符串"
}
```

优先 argv，可选 shell mode：

```json
{
  "shell": "pnpm test && pnpm lint"
}
```

shell mode 使用更严格 approval policy。

---

# 22. Process Execution 内部流程

```text
validate cwd
 ↓
resolve executable
 ↓
classify command
 ↓
calculate permissions
 ↓
sandbox policy
 ↓
approval engine
 ↓
spawn process group
 ↓
stream stdout/stderr
 ↓
bounded ring buffer
 ↓
spill to artifact file if needed
 ↓
wait / timeout / cancel
 ↓
kill process tree
 ↓
return result
```

---

# 23. Job Output 内存策略

禁止：

```rust
Vec<u8> // 无限 append
```

采用：

```text
stdout/stderr
    │
    ├─ recent ring buffer: 256 KiB
    │
    └─ artifact file: disk
```

返回模型：

```json
{
  "exit_code": 1,
  "stdout_tail": "...",
  "stderr_tail": "...",
  "truncated": true,
  "artifact_id": "art_xxx"
}
```

如果 ChatGPT 需要更多：

```json
{
  "tool": "job",
  "action": "read_output",
  "id": "job_xxx",
  "stream": "stderr",
  "offset": 0,
  "limit": 65536
}
```

---

# 24. Tool 6：job

后台命令：

```json
{
  "tool": "exec",
  "argv": ["pnpm", "dev"],
  "background": true
}
```

返回：

```json
{
  "job_id": "job_01...",
  "status": "running"
}
```

后续：

```json
{
  "action": "poll",
  "id": "job_01..."
}
```

或：

```json
{
  "action": "cancel",
  "id": "job_01..."
}
```

Job 状态：

```text
starting
running
exited
failed
timed_out
cancelled
```

不要一开始实现 WebCodex 那种复杂 recovery graph。

---

# 25. Job 生命周期策略

V1：

```text
Job lifetime <= Host process lifetime
```

Host 正常退出：

```text
kill owned process groups
```

后台任务可以在 ChatGPT 多个 turn 之间继续。

但 Host 重启后：

```text
不会尝试接管旧进程
```

这是有意设计。

P2 如果确实需要 detached durable jobs，再设计独立机制。

不要提前引入复杂 durable job adoption。

---

# 26. PTY 策略

大多数 coding-agent 命令不需要完整 terminal emulator。

V1：

```text
pipe stdout/stderr
```

P1 增加 optional pseudo-PTY：

```text
feature = "pty"
```

用于：

- 某些 dev server；
- CLI 颜色/TTY 行为；
- 少数 interactive command。

不要为了 PTY 把整个 terminal stack 常驻。

---

# 27. Tool 7：git

统一 gateway：

```json
{
  "action": "status"
}
```

```json
{
  "action": "diff",
  "staged": false
}
```

```json
{
  "action": "log",
  "limit": 20
}
```

```json
{
  "action": "show",
  "revision": "HEAD"
}
```

第一阶段不要暴露：

```text
push
force-push
remote mutation
credential management
```

Commit 可 P1 增加，但需要 approval。

Git 使用系统：

```text
git
```

不使用 libgit2，原因：

- 系统 Git 行为用户熟悉；
- 内存更低；
- 减少 native dependency；
- credential/helper 行为更一致；
- 无需重新实现大量 Git 边缘语义。

---

# 28. Tool 8：permission

Permission/Approval 必须结构化。

命令命中 `Ask`：

```json
{
  "status": "approval_required",
  "approval": {
    "id": "apr_xxx",
    "summary": "Run network-enabled command",
    "reason": "...",
    "expires_at": "...",
    "scope": {
      "cwd": "/Users/me/project",
      "argv_hash": "..."
    }
  }
}
```

ChatGPT 应停止执行并询问用户。

下一 turn：

```json
{
  "action": "approve",
  "id": "apr_xxx"
}
```

执行前重新验证：

```text
ticket exists
not expired
same command hash
same cwd
same requested permissions
same workspace
```

禁止：

```text
approve A
execute B
```

---

# 29. Approval Policy

三值：

```text
Allow
Ask
Deny
```

V1 内置规则。

## 默认 Allow

示例：

```text
rg
git status
git diff
git log
npm test
pnpm test
cargo test
pytest
eslint
tsc
```

注意：

不能只按第一个 token 盲目 allow。

需要考虑：

```text
git diff --no-index /secret ...
```

因此 Allow 规则还应受：

```text
cwd
path args
network
write behavior
```

约束。

## 默认 Ask

例如：

```text
network access
package install
git commit
file writes outside workspace
kill unrelated process
shell pipeline with unknown effects
```

## 默认 Deny

例如：

```text
system directory destructive writes
credentials directories
workspace policy bypass
approval ticket forgery
```

---

# 30. Exec Policy 不直接复用完整 Codex execpolicy

Codex 当前 execpolicy 支持 Starlark 等更复杂机制。

Lite Host 第一版仅需要：

```rust
enum Decision {
    Allow,
    Ask(Reason),
    Deny(Reason),
}
```

和简单规则：

```text
prefix matcher
executable matcher
path matcher
network flag
workspace write flag
```

P2 再增加：

```text
rules.toml
```

例如：

```toml
[[rule]]
command = ["pnpm", "test"]
decision = "allow"

[[rule]]
command_prefix = ["curl"]
decision = "ask"
```

不要引入语言 runtime。

---

# 31. macOS Sandbox

macOS 是第一目标平台。

Sandbox API 应抽象：

```rust
trait SandboxBackend {
    async fn spawn(
        &self,
        request: SandboxSpawnRequest
    ) -> Result<Child>;
}
```

实现：

```text
MacOsSeatbeltBackend
NoSandboxBackend
```

策略模型：

```text
read-only
workspace-write
danger-full-access
```

默认：

```text
workspace-write
```

允许：

```text
workspace root      rw
/tmp                rw
$TMPDIR             rw
必要工具链路径        read/execute
```

拒绝：

```text
其它用户目录写
~/.ssh 写
~/.aws 写
系统目录写
未授权 network
```

注意：

实现必须参考当前 Codex macOS sandbox 行为，但不能盲目复制上游所有跨平台依赖。

---

# 32. Sandbox 与 Structured Tools 必须共享权限内核

这是整个安全设计最重要的不变量之一。

错误架构：

```text
exec -> sandbox policy
patch -> its own path check
write -> another path check
git -> no check
```

正确：

```text
                PermissionEngine
               /       |        \
              /        |         \
          patch       exec       git
            |           |         |
       PathPolicy   Sandbox   GitPolicy
```

即：

```text
一份 authority
多个执行后端
```

---

# 33. Workspace 注册

V1 不做“扫描全电脑”。

启动显式指定：

```bash
web-harness-host serve \
  --workspace /Users/me/code/project
```

或者配置：

```toml
[workspace]
root = "/Users/me/code/project"
```

Host 只能访问：

```text
workspace root
+
明确允许的 additional_roots
```

后续 P1：

```text
registered projects
```

但仍由用户显式注册。

---

# 34. Config 设计

推荐：

```text
~/.config/web-harness/config.toml
```

示例：

```toml
version = 1

[workspace]
root = "/Users/me/code/project"

[security]
sandbox = "workspace-write"
network = "ask"
write_outside_workspace = "deny"

[resources]
profile = "low-memory"
max_foreground_processes = 2
max_background_jobs = 2
job_ring_buffer_kib = 256
max_read_total_kib = 512

[search]
backend = "rg"
max_results = 200

[state]
enabled = true
path = "~/.local/state/web-harness/state.db"
```

Secrets 不写这里。

Tunnel API key 由 `tunnel-client` 自己管理。

---

# 35. State

可以使用 SQLite，但必须非常薄。

SQLite 是 embedded library，不是数据库 daemon。

保存：

```text
projects
sessions
approval_tickets
job metadata
recent tool metadata
```

不保存：

```text
完整 ChatGPT 对话
model reasoning
长期 memory
agent graph
token history
```

因为这些属于 ChatGPT。

---

# 36. Session

Session 只维护执行连续性。

```rust
struct Session {
    id: SessionId,
    workspace: PathBuf,
    sandbox_mode: SandboxMode,
    changed_paths: BTreeSet<PathBuf>,
    active_jobs: BTreeSet<JobId>,
    last_git_head: Option<String>,
}
```

不要把 session 演化成：

```text
Agent
Conversation
Goal
Task
Attempt
Run
Delivery
Wake
```

这是 WebCodex 类型平台架构，不符合 Lite Host。

---

# 37. External MCP 支持

如果未来要接近 Codex 的扩展能力，可以让 Host 作为轻量 MCP client 调用其它本地 MCP。

但：

```text
P1 optional feature
```

结构：

```text
ChatGPT
  ↓
web-harness-host
  ↓
extension gateway
  ↓
third-party MCP
```

默认不开。

禁止 V1 自动启动一堆 MCP server。

---

# 38. Tool Schema 设计原则

模型 tool schema 直接影响：

- Prompt token；
- tool selection；
- latency；
-错误率。

因此：

## 做

```text
少量顶层 tools
action discriminators
紧凑结果
批量操作
parser-ready continuation
```

## 不做

```text
50+ tools
重复 aliases
大量 timestamp
内部 lifecycle metadata
implementation detail
超大 schema descriptions
```

---

# 39. Tool Result 设计

成功结果必须：

```text
foreground business truth
```

例如 search：

```json
{
  "matches": [
    {
      "path": "src/a.ts",
      "line": 34,
      "text": "..."
    }
  ],
  "truncated": false
}
```

不要：

```json
{
  "request_started_at": "...",
  "dispatcher_version": "...",
  "worker_id": "...",
  "absolute_cursor": "...",
  "transport": "...",
  "audit_internal": "..."
}
```

这些可以日志记录，不应该进入模型上下文。

---

# 40. Error Contract

统一：

```json
{
  "error": {
    "code": "path_outside_workspace",
    "message": "Path is outside the allowed workspace",
    "retryable": false,
    "next": null
  }
}
```

常见 code：

```text
invalid_request
path_not_found
path_outside_workspace
binary_file
read_limit_exceeded
patch_parse_error
patch_conflict
permission_denied
approval_required
command_not_found
command_timeout
job_not_found
git_error
resource_limit
sandbox_error
internal_error
```

不要让模型依赖 stderr 文本来判断错误类型。

---

# 41. 性能核心：减少 Tunnel Round Trip

真正端到端瓶颈主要不是 Rust 本地函数几毫秒。

而是：

```text
model
↓
OpenAI scheduling
↓
Tunnel
↓
local
↓
Tunnel
↓
model
```

因此关键优化优先级：

```text
1. 减少 tool call 数
2. 缩小 tool schema
3. 缩小 tool result
4. batch read/search
5. 避免不必要 discovery
6. 本地 dispatch 性能
```

不是反过来。

---

# 42. Batch Read

错误：

```text
read a.ts
read b.ts
read c.ts
read package.json
```

正确：

```text
read [a.ts,b.ts,c.ts,package.json]
```

一次往返。

---

# 43. Search + Read Fusion

P1 可以支持：

```json
{
  "action": "search_and_read",
  "query": "createClient",
  "top_files": 5,
  "max_lines_per_file": 120
}
```

但不要 V1 过度组合所有工具。

先根据真实 trace 判断：

```text
哪些连续调用最常见
```

再合并。

---

# 44. 资源 Governor

实现：

```rust
struct ResourceLimits {
    max_foreground_processes: usize,
    max_background_jobs: usize,
    max_output_buffer_bytes: usize,
    max_read_total_bytes: usize,
    max_search_results: usize,
}
```

低内存 profile：

```text
foreground exec = 2
background jobs = 2
job memory buffer = 256 KiB
search results = 200
read total = 512 KiB/call
LSP = disabled
```

---

# 45. 内存策略

原则：

```text
bounded everything
```

包括：

- 读取文件；
- search result；
- stdout；
- stderr；
- MCP response；
- patch input；
- diff；
- job count；
- session count；
- instruction cache。

不得出现：

```text
“理论上通常不会太大”
```

必须有硬限制。

---

# 46. LSP

V1：

```text
不实现
```

因为：

```text
rg + git + read
```

已经足以完成大量 coding agent 工作。

P2：

```text
lazy LSP
```

生命周期：

```text
semantic request
   ↓
no LSP
   ↓
spawn
   ↓
request
   ↓
idle TTL 120s
   ↓
kill
```

低内存 profile：

```text
max_lsp_processes = 1
```

---

# 47. File Watcher

默认：

```text
disabled
```

如果用户正在 IDE 中同时修改：

每个 tool call 重新从 filesystem 获取需要的数据。

这比常驻 watcher：

- 更简单；
- 更省内存；
- 更少 race；
- 更容易保持 source of truth。

---

# 48. UI

核心产品不依赖 UI。

命令：

```bash
web-harness doctor
web-harness status
web-harness config
```

应足够完成大部分运维。

P2 可提供：

```bash
web-harness ui
```

才临时启动：

```text
127.0.0.1:<ephemeral>
```

UI 使用：

```text
Vite static build
```

release 时 embed 进 Host：

```rust
include_bytes!
```

或类似方案。

UI 不进入 MCP data path。

---

# 49. UI 不能成为运行依赖

错误：

```text
ChatGPT
 ↓
Tunnel
 ↓
HTTP API
 ↓
UI backend
 ↓
Runtime
```

正确：

```text
ChatGPT
 ↓
Tunnel
 ↓
Runtime

Browser UI
 ↓
optional localhost diagnostics
 ↓
Runtime
```

UI 挂掉不能影响 ChatGPT coding。

---

# 50. CLI

建议：

```text
web-harness init
web-harness serve --stdio
web-harness doctor
web-harness config show
web-harness workspace check
web-harness benchmark
web-harness version
```

## init

只生成本机 config。

不负责创建 OpenAI Tunnel。

OpenAI Tunnel 保持官方流程，减少未来协议维护风险。

---

# 51. OpenAI Tunnel 配置流程

官方流程建议保持独立。

示意：

```bash
export CONTROL_PLANE_API_KEY="..."
```

创建 profile：

```bash
tunnel-client init \
  --sample sample_mcp_stdio_local \
  --profile web-harness \
  --tunnel-id tunnel_xxx \
  --mcp-command "/usr/local/bin/web-harness serve --stdio"
```

检查：

```bash
tunnel-client doctor \
  --profile web-harness \
  --explain
```

运行：

```bash
tunnel-client run \
  --profile web-harness
```

然后在 ChatGPT developer-mode app 中选择：

```text
Connection: Tunnel
```

---

# 52. 启动流程

```text
tunnel-client starts
    ↓
spawn web-harness-host
    ↓
Host loads config
    ↓
validate workspace
    ↓
build capability table
    ↓
start MCP stdio
    ↓
initialize
    ↓
tools/list
    ↓
ready
```

Host 启动时禁止：

```text
recursive repository scan
git status -uall huge scan
LSP start
file index
AST parse
dependency graph
```

这些必须 lazy。

---

# 53. Shutdown

收到：

```text
SIGTERM / Ctrl-C / parent pipe close
```

流程：

```text
stop accepting tools
 ↓
cancel foreground commands
 ↓
kill owned job process groups
 ↓
flush tiny state
 ↓
exit
```

超时：

```text
graceful timeout 2–5s
```

之后强制 kill child process group。

---

# 54. 日志

默认：

```text
stderr human logs
```

支持：

```text
RUST_LOG
```

但 release 默认：

```text
warn
```

不要 debug 常驻刷盘。

可选：

```bash
--log-file
```

做 rotation：

```text
10 MiB × 3
```

---

# 55. Observability

只保留本机需要的：

```text
startup time
tool latency histogram
tool errors
child process count
job count
RSS sample
```

不默认上传 telemetry。

`web-harness doctor` 输出：

```text
Host: OK
Workspace: OK
rg: OK
git: OK
Sandbox: OK
Tunnel: checked externally
Memory profile: low-memory
```

---

# 56. 安全边界

Trust Boundary：

```text
ChatGPT / Remote Model
      │ untrusted instructions
      ▼
MCP Tool Contract
      │
      ▼
Permission Engine
      │
 ┌────┴─────┐
 ▼          ▼
Files      Exec
 │          │
PathGuard  Sandbox
 └────┬─────┘
      ▼
Local Workspace
```

模型永远不是本地 authority。

---

# 57. Prompt Injection 防线

本地代码本身也可能包含恶意 instruction。

Host 不应该：

```text
读取 README 后自动扩大权限
```

AGENTS 只影响：

```text
开发约定 / coding behavior
```

不能影响：

```text
filesystem authority
sandbox
approval
network
workspace roots
```

即：

```text
Instruction != Permission
```

这是必须写进架构测试的不变量。

---

# 58. Secrets

默认 deny/ask：

```text
~/.ssh
~/.aws
~/.gnupg
Keychain exports
browser profiles
.env outside workspace
```

Workspace 内 `.env` 是否允许读取：

建议默认：

```text
ask / configurable
```

或者提供：

```toml
[security.secrets]
workspace_dotenv = "ask"
```

工具结果必须支持 redaction hook。

---

# 59. Git Dirty Workspace

Host 不得自动：

```text
git reset --hard
git checkout .
clean -fd
```

patch 前无需要求 clean workspace。

但是返回：

```text
preexisting_changes
```

并避免覆盖与当前任务无关的用户修改。

---

# 60. Concurrency

MCP Tool 允许有限并发。

规则：

```text
read/search:
  concurrent

patch/git mutation:
  workspace write lock

exec:
  resource governor

approval:
  serialized per ticket
```

实现：

```rust
RwLock<WorkspaceMutationState>
Semaphore<ExecSlots>
```

不要全局 Mutex 所有 tool。

---

# 61. File Mutation Fence

对 patch/write：

```text
optimistic content fence
```

patch parse 后记录：

```text
mtime
size
optional hash
```

真正 commit 前再次检查。

如果文件已被 IDE 修改：

```text
patch_conflict
```

而不是覆盖。

---

# 62. Git 与 Patch 的一致性

Patch 成功返回：

```json
{
  "changed_paths": [
    "src/a.ts"
  ],
  "summary": "...",
  "git_diff_available": true
}
```

不自动运行：

```text
git diff entire repo
```

如果 patch 很大，仅返回 compact summary。

需要详细 diff 时让 ChatGPT 调：

```text
git(action=diff)
```

降低 response token。

---

# 63. MCP Protocol 实现

不要自己从零写 JSON-RPC parser，如果成熟 Rust MCP SDK 已满足：

- stdio transport；
- tools/list；
- tools/call；
- protocol version negotiation；
- cancellation。

优先使用 SDK。

但：

```text
MCP framework
```

不能控制业务 authority。

Tool Handler 只做：

```text
decode -> typed request -> kernel
```

核心业务逻辑应可脱离 MCP 单测。

---

# 64. Host Kernel

建议：

```rust
struct HostKernel {
    workspace: Workspace,
    permissions: PermissionEngine,
    jobs: JobManager,
    resources: ResourceGovernor,
    state: StateStore,
}
```

MCP handler：

```rust
async fn call_tool(req: ToolCall) -> ToolResult {
    kernel.dispatch(req).await
}
```

不要把所有逻辑塞进：

```text
tools/call handler
```

---

# 65. Typed Tool Request

```rust
enum ToolCall {
    Workspace(WorkspaceRequest),
    Read(ReadRequest),
    Search(SearchRequest),
    Patch(PatchRequest),
    Exec(ExecRequest),
    Job(JobRequest),
    Git(GitRequest),
    Permission(PermissionRequest),
}
```

Parse 一次。

后续：

```text
audit
permission
dispatch
```

都使用 typed request。

不要反复解析 `serde_json::Value`。

---

# 66. Git Gateway 示例类型

```rust
enum GitAction {
    Status,
    Diff {
        staged: bool,
        pathspec: Vec<PathBuf>,
    },
    Log {
        limit: u32,
    },
    Show {
        revision: String,
    },
}
```

不要直接：

```rust
git(args: Vec<String>)
```

否则权限边界最终会退化成 shell。

---

# 67. Exec 类型

```rust
struct ExecRequest {
    argv: Vec<String>,
    cwd: Option<PathBuf>,
    env: BTreeMap<String, String>,
    timeout_ms: Option<u64>,
    background: bool,
    shell: bool,
    permissions: RequestedPermissions,
}
```

上限：

```text
argv count
arg length
env count
env value size
timeout max
```

全部必须 bounded。

---

# 68. Resource Profile

提供：

```text
low-memory
balanced
unrestricted
```

## low-memory

```text
exec slots = 2
background = 2
LSP = off
read = 512KiB
ring buffer = 256KiB/job
session cache = small
search max = 200
```

## balanced

```text
exec slots = 4
background = 4
LSP optional
read = 2MiB
ring buffer = 512KiB
```

---

# 69. Binary Size

Release：

```toml
[profile.release]
strip = "symbols"
lto = "thin"
codegen-units = 1
panic = "abort" # 仅 benchmark 后决定
```

是否 `panic=abort`：

必须经过：

- crash diagnostics；
- child cleanup；
- binary size；

综合验证后再决定。

不要为了几 MB 破坏可恢复性。

---

# 70. Dependency Budget

新增依赖必须说明：

```text
为什么需要
binary size delta
RSS delta
compile time delta
transitive deps
security surface
```

PR 模板增加：

```text
### Dependency impact
- New runtime dependency:
- Why:
- Alternatives considered:
- Binary size delta:
- Idle RSS delta:
```

---

# 71. 性能 Benchmark

必须建立：

```text
cargo bench / custom harness
```

场景：

## Bench A：idle

```text
start
wait 30s
measure RSS
measure CPU
child process count
```

## Bench B：1000 MCP noop dispatch

测：

```text
p50
p95
p99
allocation
```

## Bench C：read

```text
1 file
10 files
100 files
```

## Bench D：rg

```text
small repo
medium repo
large repo
```

## Bench E：job output

生成：

```text
1 MB
10 MB
100 MB
```

确认 RSS 不随输出线性增长。

---

# 72. 必测真实机器

至少：

```text
Apple Silicon 8 GB
Intel Mac 8 GB
```

建议 CI 无法模拟 RSS 时：

```text
nightly physical benchmark
```

保存趋势：

```text
benchmarks/history.json
```

PR regression threshold：

```text
idle RSS +15% fail/warn
dispatch p95 +20% warn
```

---

# 73. Testing Strategy

分五层。

## Layer 1：Unit

```text
PathGuard
Patch parser
Approval ticket
Rule matching
Output ring buffer
AGENTS scope
```

## Layer 2：Property

重点：

```text
path traversal
symlink escape
patch arbitrary input
UTF-8 edge
line endings
approval hash
```

## Layer 3：Integration

使用 temp repo：

```text
read
patch
git diff
exec test
job
```

## Layer 4：Sandbox

确认：

```text
workspace write success
outside write fail
network policy
/tmp behavior
symlink escape fail
```

## Layer 5：Tunnel E2E

真实：

```text
ChatGPT
↓
OpenAI Tunnel
↓
Host
↓
fixture repo
```

---

# 74. Security Test Cases

必须有：

```text
../../etc/passwd
workspace/symlink -> ~/.ssh
patch path traversal
git pathspec escape
shell redirect outside workspace
shell via child process write outside
approval reuse
expired approval
approval payload mutation
huge stdout
huge patch
huge read
binary read
malformed UTF-8
process tree orphan
timeout kill
```

---

# 75. Fuzzing

重点 fuzz：

```text
patch parser
MCP input decode
path normalization
approval token decode
search JSON parse
```

可以：

```text
cargo fuzz
```

Patch parser尤其值得 fuzz。

---

# 76. 开发阶段

---

## Phase 0：Skeleton

目标：

```text
可编译、可测试、边界清晰
```

任务：

1. Cargo workspace；
2. lint；
3. fmt；
4. CI；
5. protocol crate；
6. Host stdio skeleton；
7. architecture boundary test；
8. benchmark skeleton。

验收：

```text
MCP initialize
tools/list
health test
```

---

## Phase 1：Workspace + Read + Search

实现：

```text
PathGuard
workspace root
AGENTS discovery
batch read
rg search
rg --files
git ls-files
```

验收：

ChatGPT 可以：

```text
“分析这个项目的前端入口”
```

并正确批量读取真实本机文件。

---

## Phase 2：Patch

实现：

```text
Codex-derived patch parser
validation
path permissions
atomic write
conflict fence
compact response
```

验收：

ChatGPT：

```text
修改 3 个文件
```

一次 patch 成功，并在本机 IDE 立即看到变化。

---

## Phase 3：Exec

实现：

```text
argv exec
timeout
stdout/stderr
process group
kill tree
resource governor
```

验收：

```text
pnpm test
cargo test
pytest
```

真实执行。

---

## Phase 4：Jobs

实现：

```text
background
poll
read_output
cancel
disk spill
```

验收：

```text
pnpm dev
```

启动后 ChatGPT 下一轮仍能检查状态。

---

## Phase 5：Git

实现：

```text
status
diff
log
show
```

验收：

ChatGPT 可以在修改后 review 自己的 diff。

---

## Phase 6：Sandbox + Approval

实现：

```text
PermissionEngine
macOS Seatbelt
Allow/Ask/Deny
approval ticket
network policy
```

这是 Production Gate。

未完成前：

```text
只能标记 Developer Preview
```

---

## Phase 7：Secure Tunnel Productization

实现文档和 installer：

```text
tunnel profile
doctor
ChatGPT app setup
first read test
```

验收：

从干净 Mac：

```text
安装
配置
ChatGPT 读取 README
ChatGPT 修改 fixture
```

全流程。

---

## Phase 8：Low-Memory Hardening

专门优化：

```text
heap profile
RSS
copy
buffer
dependencies
Tokio
job output
response size
```

任何“感觉很轻”都不算完成。

必须 benchmark。

---

## Phase 9：Optional UI

只有 Runtime 稳定后再做。

功能：

```text
Status
Workspace
Tunnel hints
Jobs
Approvals
Logs
Settings
```

不做：

```text
code editor
terminal emulator
Chat UI
```

这些已有 IDE / ChatGPT。

---

## Phase 10：Optional LSP / Extensions

基于真实 trace 决定。

---

# 77. 原子提交策略

每个 commit：

```text
一个完整语义变化
```

示例：

```text
feat(protocol): add typed MCP tool requests
feat(workspace): enforce canonical root guard
feat(search): add bounded ripgrep backend
feat(patch): add Codex-compatible parser
feat(exec): add process-group execution
feat(jobs): add bounded output artifacts
feat(sandbox): add macOS workspace-write profile
feat(approval): bind tickets to command digest
perf(read): batch filesystem reads
```

不要：

```text
feat: finish everything
```

---

# 78. Branch / Review Gate

每 Phase：

```text
implementation
tests
docs
benchmark
security review
```

合并条件：

```text
cargo fmt
cargo clippy
cargo test
security tests
dependency audit
benchmark smoke
```

---

# 79. CI

GitHub Actions：

```text
check
test
clippy
fmt
audit
deny
fuzz-smoke
build-macos
build-linux
```

后期：

```text
release artifacts
SBOM
checksums
provenance
```

---

# 80. Supply Chain

建议：

```text
cargo-deny
cargo-audit
SBOM
SHA256SUMS
GitHub artifact attestation
```

对于 Codex-derived 文件：

CI 检查：

```text
third-party attribution exists
```

---

# 81. Release

macOS：

```text
web-harness-aarch64-apple-darwin
web-harness-x86_64-apple-darwin
```

第一阶段：

```text
Homebrew tap
```

后期：

```text
notarized pkg/dmg
```

但不需要 Desktop shell。

`tunnel-client` 继续使用 OpenAI 官方安装渠道，不内嵌复制它的 binary，除非未来有强需求并正确处理更新/签名。

---

# 82. 更新策略

Host：

```text
独立版本
```

例如：

```text
0.1.x Developer Preview
0.2.x Sandbox Beta
1.0 Stable
```

Tunnel：

```text
不固定复制旧版
```

文档指向 OpenAI 官方当前支持版本。

---

# 83. 兼容性策略

本项目对 Secure Tunnel 的依赖仅：

```text
standard MCP server over stdio
```

因此即使未来 tunnel-client 内部 transport 改变，只要 stdio MCP contract 保持支持，Host 不需要跟着重构。

这是选择：

```text
official sidecar + standard boundary
```

而不是重写 OpenAI tunnel protocol 的核心原因。

---

# 84. 为什么不自己实现 Secure Tunnel Client

即使 OpenAI tunnel-client 开源，也不建议 Rust 重写。

原因：

- auth；
- long-poll；
- organization/workspace association；
- tunnel protocol；
- control plane changes；
- mTLS；
- retries；
- health；
- future compatibility；

都属于 OpenAI transport domain。

本项目没有必要拥有。

---

# 85. 为什么不嵌入 Go SDK

OpenAI 提供 Go SDK 可以 in-memory transport。

但本项目 Host 为 Rust。

三种方案：

```text
A. Rust FFI Go SDK
B. 把 Host 改成 Go
C. tunnel-client sidecar + stdio
```

最终选：

```text
C
```

因为：

- 标准边界；
- 解耦升级；
- 无 FFI；
- 开发快；
- 崩溃隔离；
- 两个小型原生进程的资源成本可接受。

如果未来 benchmark 证明 `tunnel-client` + Host 的 stdio 成为显著瓶颈，再重新评估；不要提前优化。

---

# 86. 与 WebCodex 的区别

WebCodex 更适合：

```text
multi-runner
multi-machine
durable workflow
server routing
enterprise-like operation
```

本项目：

```text
single local machine
single user
ChatGPT owns conversation
Host owns local execution only
```

因此删除：

```text
Server
Runner registry
QUIC bridge
Workflow Session system
Durable Agent
Goal domain
multi-user auth
```

主路径从：

```text
ChatGPT
→ Tunnel
→ Server
→ Runner
→ Project
```

缩短为：

```text
ChatGPT
→ Tunnel
→ Host
→ Project
```

---

# 87. 与“直接 MCP filesystem server”的区别

普通 filesystem MCP：

```text
read
write
list
```

不足以获得 Codex 体验。

本项目还需要：

```text
AGENTS
patch
exec
jobs
sandbox
approval
git
process lifecycle
resource governor
compact continuity
```

这就是“工具服务器”和“Codex-style local runtime”的区别。

---

# 88. 与“ChatGPT → codex exec”的区别

不采用：

```text
ChatGPT
→ MCP
→ codex exec
→ OpenAI Model
→ tools
```

因为：

```text
模型套模型
agent 套 agent
```

严重增加：

- latency；
- token；
- unpredictability；
- debug complexity。

本项目只复用 Codex **本地 execution semantics**。

---

# 89. 第一版 Tool Capability Definition

最终 MVP：

```text
workspace
  info
  instructions

read
  files

search
  content
  files

patch
  apply

exec
  foreground
  background

job
  poll
  output
  cancel

git
  status
  diff
  log
  show

permission
  approve
  deny
```

足够形成 coding loop。

---

# 90. ChatGPT 实际开发流程示例

用户：

```text
帮我修复登录页面刷新后 token 丢失的问题
```

ChatGPT：

```text
workspace(info)
```

Host：

```text
project metadata
```

ChatGPT：

```text
search([
  "auth",
  "token",
  "localStorage"
])
```

ChatGPT：

```text
read([
  "src/auth/store.ts",
  "src/router.ts",
  "src/api/client.ts"
])
```

ChatGPT：

```text
patch(...)
```

ChatGPT：

```text
exec(["pnpm","test"])
```

失败：

```text
stderr tail
```

ChatGPT：

```text
read(...)
patch(...)
exec(...)
git(diff)
```

最终：

```text
完成
```

整个过程中：

```text
没有第二个 Agent
没有云端 repo 副本
没有 Runner
没有 Node Host
```

---

# 91. 最小 Product UX

安装：

```bash
brew install .../web-harness
brew install openai/tools/tunnel-client
```

初始化：

```bash
cd my-project
web-harness init .
```

配置 Tunnel：

```bash
tunnel-client init ...
```

运行：

```bash
tunnel-client run --profile web-harness
```

ChatGPT：

```text
Settings / Plugins / developer-mode app
→ Tunnel
→ choose tunnel
```

完成。

---

# 92. 自动生成 Tunnel Profile

项目 CLI 可以输出建议命令：

```bash
web-harness tunnel print-command
```

输出：

```text
tunnel-client init ...
```

但不直接管理：

```text
OpenAI API key
tunnel admin permissions
organization RBAC
```

遵循最小责任原则。

---

# 93. Doctor

```bash
web-harness doctor
```

检查：

```text
[OK] macOS supported
[OK] workspace readable
[OK] workspace writable
[OK] git
[OK] rg
[OK] sandbox backend
[OK] temp directory
[OK] MCP stdio self-test
[OK] resource profile
```

Tunnel 使用官方：

```bash
tunnel-client doctor
```

不要复制官方 diagnostics。

---

# 94. Self-Test

提供：

```bash
web-harness self-test --workspace ./fixture
```

执行：

```text
list
read
search
patch temp fixture
git diff
exec
cleanup
```

用于用户快速确认本机 runtime。

---

# 95. Performance Trace

debug 模式可以输出：

```text
tool=read parse=0.1ms policy=0.2ms io=1.3ms encode=0.4ms total=2.0ms
```

默认关闭。

这有助于判断：

```text
问题在 Host
还是 Tunnel
还是模型 round-trip
```

---

# 96. 数据不应进入 State DB 的内容

禁止保存：

```text
文件正文
大型 diff
stdout 全量
stderr 全量
secret
ChatGPT prompts
```

Job 大输出：

```text
temporary artifact
```

并 TTL 删除。

---

# 97. Artifact Cleanup

默认：

```text
jobs artifacts TTL = 24h
max total artifact disk = 512MB
```

low-memory/low-disk 可以：

```text
128MB
```

达到上限：

```text
LRU cleanup completed jobs
```

绝不无限增长。

---

# 98. Crash Safety

重点：

```text
不损坏文件
不遗留无限子进程
不损坏 state DB
```

文件：

```text
atomic rename
```

DB：

```text
SQLite WAL 可选
```

Child：

正常 shutdown kill group。

异常 crash 后不能保证所有第三方程序停止，因此开发阶段必须专门测试 process ownership。

---

# 99. 低内存 Mac 优先优化顺序

如果 RSS 超标，按以下顺序排查：

1. 依赖树；
2. Tokio runtime；
3. MCP SDK；
4. SQLite；
5. job buffers；
6. caches；
7. allocator；
8. debug symbols/build profile。

不要第一反应就重写全部 Rust。

---

# 100. 成功标准

项目达到 `1.0` 前必须满足：

## 功能

- [ ] ChatGPT Web 能读本地真实项目；
- [ ] ChatGPT Web 能批量搜索；
- [ ] ChatGPT Web 能安全修改；
- [ ] ChatGPT Web 能执行测试；
- [ ] ChatGPT Web 能管理后台 job；
- [ ] ChatGPT Web 能查看 Git diff；
- [ ] AGENTS.md 正确作用；
- [ ] sandbox 生效；
- [ ] approval 生效；
- [ ] 路径逃逸测试通过。

## 资源

- [ ] 8 GB Apple Silicon Mac 流畅；
- [ ] 8 GB Intel Mac 流畅；
- [ ] Idle CPU 接近 0；
- [ ] 无 Node；
- [ ] 无 Browser；
- [ ] 无常驻 LSP；
- [ ] Tunnel + Host idle RSS 达标；
- [ ] 100 MB stdout 不造成 100 MB RSS 增长。

## 体验

- [ ] 从 ChatGPT 发出任务后无需人工频繁手工切终端；
- [ ] 普通开发循环不需要管理 Host 内部概念；
- [ ] 安全操作自动执行；
- [ ] 高风险操作在 ChatGPT 中产生明确 approval；
- [ ] ChatGPT 能连续修复测试直到通过。

---

# 101. 最终技术决策表

| 项目 | 最终选择 |
|---|---|
| 上层 Agent | ChatGPT Web |
| Model Runtime | ChatGPT 自己 |
| Local Host | Rust |
| Tunnel | OpenAI 官方 tunnel-client |
| Local MCP transport | stdio |
| HTTP Server | 默认无 |
| Runner | 无 |
| Database Service | 无 |
| State | 极薄 embedded SQLite |
| Search | rg / git ls-files |
| Editing | Codex-style structured patch |
| Sandbox | macOS-first，Codex-style semantics |
| Approval | Allow / Ask / Deny |
| Git | system git |
| LSP | 默认关闭，未来 lazy |
| File Watcher | 默认关闭 |
| UI | optional / on-demand |
| Node/Bun Runtime | 无 |
| Electron | 无 |
| Browser Runtime | 无 |
| License | Apache-2.0 |
| Codex reuse | selective |
| codex-core dependency | 禁止 |
| Agent→Agent | 禁止 |
| 8GB Mac | 第一等目标 |

---

# 102. 推荐第一批开发 Issues

```text
#1  Bootstrap Rust workspace and dependency boundaries
#2  Implement MCP stdio server skeleton
#3  Implement canonical Workspace PathGuard
#4  Implement scoped AGENTS discovery
#5  Implement bounded batch read tool
#6  Implement ripgrep search backend
#7  Extract/adapt Codex patch parser with attribution
#8  Implement permission-aware patch application
#9  Implement bounded process executor
#10 Implement process-group cancellation
#11 Implement background JobManager
#12 Implement output ring buffer + disk spill
#13 Implement structured Git gateway
#14 Implement PermissionEngine
#15 Implement macOS sandbox backend
#16 Implement cryptographically bound approval tickets
#17 Add SQLite lightweight execution state
#18 Add Secure Tunnel onboarding docs
#19 Add 8GB Mac benchmark harness
#20 Add end-to-end ChatGPT tunnel acceptance suite
```

---

# 103. 开发优先级

真正的关键路径：

```text
MCP
 ↓
Workspace
 ↓
Read/Search
 ↓
Patch
 ↓
Exec
 ↓
Jobs
 ↓
Git
 ↓
Sandbox
 ↓
Approval
 ↓
Performance hardening
```

不要被：

```text
UI
LSP
multi-agent
plugins
browser
```

分散注意力。

---

# 104. ADR：最终架构决定

建议建立：

```text
docs/adr/
```

第一批：

```text
0001-use-chatgpt-as-agent.md
0002-use-openai-secure-mcp-tunnel.md
0003-use-rust-local-host.md
0004-use-stdio-transport.md
0005-no-codex-core-dependency.md
0006-selective-codex-code-reuse.md
0007-bounded-resource-policy.md
0008-single-workspace-first.md
0009-no-default-lsp-or-index.md
0010-shared-permission-kernel.md
```

防止项目半年后重新膨胀。

---

# 105. 最终原则

整个项目开发过程中，每增加一个 subsystem 都问四个问题：

```text
1. 本地 Codex 体验真的需要吗？
2. ChatGPT 已经拥有这个能力了吗？
3. 能 lazy 吗？
4. 能不用常驻服务吗？
```

如果答案是：

```text
ChatGPT 已经有
```

就不要在 Host 重做。

如果答案是：

```text
偶尔才需要
```

就 lazy。

如果答案是：

```text
只是为了“架构漂亮”
```

就不要做。

---

# 106. 最终形态

理想状态下，用户平时的 Mac 只多出：

```text
tunnel-client        轻量常驻
web-harness-host     轻量常驻
```

当 ChatGPT 真正执行开发任务时：

```text
rg
git
test/build process
```

才短暂启动。

没有任务时：

```text
CPU ≈ 0
无 repo 扫描
无 watcher
无 LSP
无 Browser
无 Node
无 Agent Runtime
```

而 ChatGPT 仍然拥有：

```text
读取
搜索
结构化修改
执行
测试
后台任务
Git
Sandbox
Approval
项目规则
```

这就是这个项目应该追求的最终平衡点：

> **不是复刻 Codex 产品，而是提炼 Codex 本地执行层最有价值的部分，把它变成 ChatGPT Web 能通过官方 Secure MCP Tunnel 安全调用的超轻量本机 Runtime。**

---

# 107. 参考与上游

## OpenAI Secure MCP Tunnel

官方文档：

https://developers.openai.com/api/docs/guides/secure-mcp-tunnels

关键事实：

- ChatGPT / Codex / Responses API 可以通过 Tunnel 使用私有 MCP；
- MCP Server 不需要公网 listener；
- tunnel-client 只需要出站 HTTPS；
- 本地 MCP 支持 stdio 或 HTTP；
- ChatGPT developer-mode app 支持 Tunnel connection；
- tunnel-client 负责 long-poll queued work 与返回 MCP response。

官方仓库：

https://github.com/openai/tunnel-client

---

## OpenAI Codex

仓库：

https://github.com/openai/codex

重点参考：

```text
codex-rs/apply-patch/
codex-rs/sandboxing/
codex-rs/execpolicy/
codex-rs/file-system/
codex-rs/file-search/
codex-rs/core/src/agents_md.rs
codex-rs/utils/absolute-path/
codex-rs/utils/path-uri/
codex-rs/utils/output-truncation/
```

但本方案明确：

```text
不依赖完整 codex-core
不嵌入 Codex Agent
不使用 Codex 再次调用模型
```

---

## Licensing

Codex：

https://github.com/openai/codex/blob/main/LICENSE

OpenAI tunnel-client：

https://github.com/openai/tunnel-client/blob/master/LICENSE

二者当前均为 Apache License 2.0。

---

# 108. 下一步可直接执行的开发顺序

如果现在开始写代码，不再继续做架构讨论，建议按以下顺序推进：

```text
Day/Stage 1
  workspace skeleton
  protocol
  stdio MCP
  path guard

Stage 2
  batch read
  rg search
  AGENTS

Stage 3
  Codex-derived patch parser
  safe apply
  diff result

Stage 4
  exec
  process group
  timeout
  bounded output

Stage 5
  JobManager
  disk artifacts

Stage 6
  git gateway

Stage 7
  PermissionEngine
  macOS Sandbox
  approval tickets

Stage 8
  official tunnel integration docs
  ChatGPT E2E

Stage 9
  memory/performance hardening

Stage 10
  optional UI / LSP only if metrics justify
```

达到 Stage 8 之后，产品已经可以进入真实日常 coding dogfood。

Stage 9 达标之后，才可以正式宣传：

```text
Designed for 8 GB Macs
```

而不是在没有 benchmark 的情况下提前宣称“极致轻量”。

---

# 109. 最终验收命令场景

发布候选版本必须能通过下列真实 ChatGPT Web 用例：

```text
“读取当前项目并说明入口”
“根据 AGENTS.md 修改这个模块”
“查找所有调用旧 API 的地方”
“批量修改这些调用”
“运行单测”
“根据失败信息继续修”
“启动 dev server”
“读取 dev server 输出”
“取消 dev server”
“给我看 git diff”
```

同时 Activity Monitor 应满足：

```text
没有 Electron
没有 Node runtime
没有 LSP server（未请求时）
没有 repo index daemon
Host + Tunnel 内存保持在 Gate 内
```

只有同时满足“能力”和“资源”两方面，项目目标才算真正完成。
