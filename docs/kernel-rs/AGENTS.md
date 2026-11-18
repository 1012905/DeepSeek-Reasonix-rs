# Reasonix Rust Kernel Rewrite — Agent Guide

> **GOAL**：使用 Rust 彻底重写 `reference/DeepSeek-Reasonix-studio` 的 Go 内核，保持 CLI、HTTP/SSE、ACP、MCP、配置、SQLite 数据与扩展协议的行为兼容；保留 React/Electron/Workers 前端不变。
>
> **Reference（只读）**：`reference/DeepSeek-Reasonix-studio/`
>
> **当前状态**：M0 已完成且本机全绿。M1 进行中：CLI 契约已冻结（`docs/contracts/cli.md`，账本 CLI-001..CLI-050）；配置契约已冻结（`docs/contracts/config.md`，账本 CONFIG-001..CONFIG-035）；HTTP 契约已冻结（`docs/contracts/http.md`，账本 ROUTE-001..ROUTE-033）；SSE 事件契约已冻结（`docs/contracts/sse.md`，账本 SSE-001..SSE-022）；ACP 契约已冻结（`docs/contracts/acp.md`，账本 ACP-001..ACP-046）；MCP 客户端契约已冻结（`docs/contracts/mcp.md`，账本 MCP-001..MCP-038）；扩展 v2 契约已冻结（`docs/contracts/ext.md`，账本 EXT-001..EXT-046）；SQLite/state 契约已冻结（`docs/contracts/state.md`，账本 STATE-001..STATE-030）；工具契约已冻结（`docs/contracts/tools.md`，账本 TOOL-001..TOOL-057）；桌面 handshake 契约已冻结（`docs/contracts/host.md`，账本 HOST-001..HOST-038）；路径/环境变量契约已冻结（`docs/contracts/paths.md`，账本 PATHS-001..PATHS-036）。M1 契约清单已全部冻结。M2 进行中：会话格式契约已冻结（`docs/contracts/session-format.md`，账本 SESS-001..SESS-100）；供应商消息模型契约已冻结（`docs/contracts/provider-message.md`，账本 PMSG-001..PMSG-020）。
>
> **本文件的语言约定**：说明与协作用中文；代码、注释、提交信息、日志、用户可见字符串使用英文（与上游 `docs/SPEC.md` 一致）。

---

## 0. 最高原则

1. **行为兼容优先于代码相似。** 不要求 Rust 代码长得像 Go；要求同样的输入产生同样的用户可见结果。
2. **契约优先于实现。** 先冻结 CLI、HTTP、SSE、ACP、MCP、配置、SQLite、扩展协议，再写实现。
3. **小步可回退优先于大爆炸。** 每个阶段都必须能独立运行、对比、回滚。
4. **安全不变量不降级。** 沙箱、权限、路径校验、SSRF、密钥处理只能更严，不能更松。
5. **前端保留。** 不重写 React、Electron、Workers、文档和发布流水线；只替换内核与两个运行时二进制。
6. **`reference/` 是 oracle，不是依赖。** 可以读、可以运行、可以录制行为；不能修改，不能把 Go 代码链接进最终产品。

---

## 1. 任务定义

### 1.1 最终交付物

最终产品必须包含：

- `reasonix`：Rust 编写的 CLI/TUI 二进制，替代 `reference/.../cmd/reasonix`。
- `reasonix-studio-host`：Rust 编写的 loopback host，替代 `reference/.../cmd/reasonix-studio-host`，供现有 Electron 桌面端连接。
- 一个 Cargo workspace，可在 Windows / macOS / Linux 上构建。
- 六个发布目标：
  - `x86_64-unknown-linux-musl`
  - `aarch64-unknown-linux-musl`
  - `x86_64-apple-darwin`
  - `aarch64-apple-darwin`
  - `x86_64-pc-windows-msvc`
  - `aarch64-pc-windows-msvc`
- 与 reference 兼容的配置、会话数据、HTTP API、SSE 事件、ACP、MCP 和扩展协议。
- 完整的 parity 账本、测试证据、性能报告和安全审查记录。

### 1.2 范围

**必须用 Rust 重写（in scope）**

- `internal/base/**`
- `internal/contract/**`
- `internal/model/**`
- `internal/runtime/**`
- `internal/session/**`
- `internal/state/**`
- `internal/tools/**`
- `internal/ext/**`
- `internal/safety/**`
- `internal/platform/**`
- `internal/assembly/**`
- `internal/frontend/**`（CLI、TUI、serve、ACP、termrender 等）
- `cmd/reasonix/**`
- `cmd/reasonix-studio-host/**`

判断标准：**只要会被链接进 `reasonix` 或 `reasonix-studio-host`，就必须是 Rust。**

**可以保留 Go（out of scope，但不得进入最终产品运行时）**

- `reference/**`：只读 oracle。
- `desktop/frontend-next/**`、`desktop/electron/**`：React/Electron 前端，保留。
- `workers/**`：Cloudflare Workers，保留。
- `sdk/go/**`：Go SDK，保留；Rust SDK 是可选后续项。
- `tools/**`：仓库 lint/coverage/price 工具，保留或后续单独移植。
- `desktop/cmd/**`：签名、manifest、update-helper 等发布工具，保留或后续单独移植。
- `cmd/e2ebench/**`：基准测试工具，保留在 reference 侧；不要为了它把 Go 内核带入产品。
- `cmd/reasonix-plugin-example/**`：示例插件，保留。
- `cmd/extension-protocol-gen/**`、`cmd/signpath-contract/**`、`cmd/studio-delta/**`：代码生成/发布辅助工具，保留或后续移植。

> 如果某个保留的 Go 工具依赖 Go 内核，让它继续针对 `reference/` 构建，或者单独移植；**不要**在 Rust 产品中恢复 Go 内核依赖。

### 1.3 非目标

- 不重写 React UI、Electron 壳、Workers、文档站、CI 发布流程。
- 不新增用户可见功能，不做 UI 改版，不扩展模型供应商范围。
- 不把协议升级到 v3；先兼容现有 ACP/MCP/extension v2。
- 不在 parity 完成前做微性能优化。
- 不追求 Rust 代码结构与 Go 包结构一一对应。
- 不把 Go 测试逐行翻译成 Rust 测试；用行为契约和 golden corpus 重建。
- 不引入 Go sidecar 作为最终形态。迁移期可用，M6 前必须移除。

### 1.4 完成定义（DoD 摘要）

内核重写完成，当且仅当：

1. `cargo build --workspace --release` 在六个目标上通过。
2. 最终二进制不依赖 Go、不依赖 cgo、不调用 reference 二进制。
3. `docs/parity/LEDGER.md` 中所有 in-scope 行均为 `done`，且每行都有测试证据。
4. CLI 命令矩阵、HTTP 路由矩阵、SSE 事件矩阵、工具目录、配置键、SQLite schema、ACP/MCP/extension 协议全部通过契约测试。
5. 现有 Electron 桌面端可以指向 Rust host 正常启动、登录、发消息、流式输出、工具审批、会话恢复。
6. Windows/macOS/Linux 三平台的核心路径通过，包括 Windows conpty、macOS seatbelt、Linux bwrap（按 reference 支持范围）。
7. 性能、内存、启动时间、二进制大小不超过第 8 节预算。
8. 安全门禁通过，沙箱/权限行为不弱于 reference。
9. 发布产物、安装包、签名/更新流程可用；桌面 smoke test 通过。
10. 文档、迁移指南、回滚方案齐全。

---

## 2. 参考基线

统计时间：2026-11-13。仅用于范围判断，不作为验收数字。

| 指标 | 数值 |
|---|---:|
| 可分析文本文件 | 10,713 |
| 文本总行数 | 1,357,659 |
| 有效代码行 | 1,120,768 |
| Go 代码 | 850,165 |
| Go 产品代码 | 358,263 |
| Go 测试代码 | 475,790 |
| 测试代码总量 | 569,435 |
| 前端/桌面产品代码 | 119,393 |
| GitHub Actions 工作流 | 28 |
| Go 模块依赖 | 69（46 direct + 23 indirect） |
| Go 版本 | 1.26.0 |

关键事实：

- 上游是 **纯 Go、`CGO_ENABLED=0`、单静态二进制**；Rust 重写应保持同样的分发体验。
- 前端是 **React SPA + Electron**，生产环境由 Go host 在同源下托管 `frontend-next/dist`。
- 后端通过 `control.Controller` 同时服务 TUI、HTTP/SSE、ACP 和 Electron。
- 最多代码集中在 `internal/frontend`、`internal/ext`、`internal/state`、`internal/platform`、`internal/contract`、`internal/runtime`、`internal/session`。
- 测试占比约 50.8%，测试重写是工作量的重要部分。

---

## 3. 硬约束（不可协商）

### 3.1 参考实现

- `reference/` **只读**。不得修改、格式化、修复或提交其中任何文件。
- 不得把 `reference/` 作为 Cargo dependency、git submodule 或构建期代码生成输入（测试期 oracle 除外）。
- 发现 reference 的 bug 时：记录到 `docs/parity/BREAKS.md`，说明影响；默认仍保持兼容，除非是安全漏洞。

### 3.2 兼容性

必须保持兼容：

- CLI 命令、子命令、flag、环境变量、退出码、stdout/stderr 结构。
- TOML 配置键、默认值、优先级、unknown key 行为、密钥存储。
- SQLite schema、迁移、WAL、锁/lease、会话 ID、时间戳语义。
- HTTP 方法、路径、状态码、JSON 字段、错误码、分页、SSE 事件名与 data 结构。
- ACP 消息与 stdio framing。
- MCP stdio/HTTP 行为、工具 schema、生命周期。
- Extension protocol v2 的 manifest、handshake、stream、UI、drain、receipt。
- 桌面 handshake：stdout 第一行 JSON 必须包含 `origin` 与 `token`，后续 `act` 行语义不变。
- 用户可见的文案、路径、默认模型、权限提示，除非明确批准变更。

### 3.3 构建与分发

- 最终二进制必须静态链接（Linux musl；Windows 使用 MSVC；macOS 使用系统库）。
- 不使用 OpenSSL；TLS 使用 rustls。
- 不依赖系统 Go、Node、Python 运行时。
- 六个目标必须在 CI 中构建；不能只保证当前开发机。
- 二进制名称、安装路径、版本输出、升级行为保持兼容。

### 3.4 安全

- 不降低 sandbox、permission、path、egress、secret 的检查强度。
- 不允许在日志、错误、崩溃报告、遥测中泄露 API key、token、cookie、用户文件内容。
- 所有网络请求必须经过现有代理/egress/SSRF 规则。
- 所有子进程必须受现有超时、输出上限、环境清理、工作目录约束。
- `unsafe` 只允许在 `reasonix-platform` / `reasonix-safety` 的受审代码中使用；每个 block 必须有 `// SAFETY:` 说明。

### 3.5 工程

- Rust stable，edition 2024，`rust-toolchain.toml` 固定版本。
- `cargo fmt`、`cargo clippy -D warnings`、`cargo test`、`cargo deny`、`cargo audit` 必须通过。
- 不提交 `target/`、密钥、真实用户数据、超过 1 MB 的测试夹具。
- 不引入会显著拖慢编译或扩大攻击面的依赖；新依赖需要说明理由。

### 4.2 依赖方向（必须无环）

```text
apps
  -> assembly
      -> cli / tui / serve / acp
      -> session
      -> agent
      -> ext
      -> platform

session -> agent -> {tools, provider, state, safety, contract}
ext     -> {contract, platform, state}
state   -> {contract, platform}
tools   -> {contract, safety, platform}
safety  -> {contract, platform}
platform-> {contract, core}
contract-> core
core    -> (nothing)
testkit -> (test-only; may depend on everything)

可以依据直接情况更改，若更改，请及时同步AGENTS.md
```

## 5. 契约优先

### 5.1 必须先冻结的契约

在写实现前，先建立以下 inventory，并以 reference 源码和实际输出为准：

| 契约 | Reference 来源 | Rust 落点 |
|---|---|---|
| CLI 命令/flag/exit code | `internal/frontend/cli/**`、`docs/CLI.md` | `reasonix-cli` |
| TOML 配置与优先级 | `internal/contract/config/**`、`reasonix.example.toml` | `reasonix-config` |
| HTTP 路由/状态码/JSON | `internal/frontend/serve/**`、`desktop/frontend-next/vite.config.ts` 的 ROUTES | `reasonix-serve` |
| SSE 事件 | `internal/contract/event/**`、`serve/eventstream.go` | `reasonix-core` / `reasonix-serve` |
| ACP | `docs/ACP.md`、`internal/frontend/acp/**` | `reasonix-acp` |
| MCP | `internal/ext/plugin/**` | `reasonix-ext` |
| Extension v2 | `docs/EXTENSION_RUNTIME_V2.md`、`internal/ext/extension/**` | `reasonix-ext` |
| SQLite schema/迁移 | `internal/state/**`、`internal/contract/config` | `reasonix-state` |
| 工具目录/schema | `internal/tools/**`、`internal/contract/tool/**` | `reasonix-tools` |
| 桌面 handshake | `cmd/reasonix-studio-host/main.go`、`desktop/electron/src/host.js` | `apps/reasonix-studio-host` |
| 路径/环境变量 | `docs/CONFIG_PATHS.md`、`internal/platform/**` | `reasonix-platform` |

### 5.2 Parity Ledger

创建 `docs/parity/LEDGER.md`，每行一个可验收行为：

```markdown
| ID | Area | Reference | Rust | Status | Evidence |
|---|---|---|---|---|---|
| CLI-001 | CLI | `reasonix --help` | `reasonix-cli` | todo | pending |
| ROUTE-012 | HTTP | `GET /status` | `reasonix-serve` | todo | pending |
| TOOL-003 | Tool | `read_file` | `reasonix-tools` | todo | pending |
```

规则：

- 所有 in-scope 行为必须有 ID。
- `done` 必须填写测试名或命令，不接受“看起来一样”。
- `blocked` 必须写原因和 owner。
- `intentional-break` 必须链接 ADR 和用户可见影响说明。
- 发布前 `todo`/`wip` 必须为 0（或明确移出范围）。
- `xtask parity --check` 必须能检查账本完整性和状态。

### 5.3 Golden corpus

建立 `tests/golden/`：

- 每个 case 包含 `input/`、`expected/`、`meta.json`。
- 用 reference 在隔离的 `REASONIX_HOME` 中录制，不接触真实用户目录。
- 规范化时间戳、UUID、端口、临时路径、模型输出。
- 流式场景比较事件序列，不比较原始字节。
- 不包含真实 API key、用户文件、大型二进制。
- mock provider 优先；测试禁止访问真实模型 API。
- 对变更使用 `insta` review，不手工改 expected 来“修绿”。

---

## 6. 迁移策略

### 6.1 Sidecar + Strangler

推荐结构：

```text
Electron / React SPA
        |
        v
reasonix-studio-host (Rust, loopback HTTP/SSE/WS)
        |
        +-- 已迁移模块 -> Rust crates
        |
        +-- 未迁移模块 -> 迁移期 sidecar: reasonix-legacy (Go, stdio JSON-RPC)
```

规则：

- 生产默认走 Rust；未实现路径才回退 Go。
- sidecar 只在迁移期存在，`REASONIX_RS_LEGACY=1` 显式开启。
- M6 前删除 sidecar 与 Go 构建依赖。
- 不允许用 FFI 传递复杂对象；进程边界 + JSON-RPC/stdio 更安全。
- 共享 SQLite 时只允许一个写者；使用 WAL、lease、幂等迁移。
- 任何 sidecar 调用都必须有超时、取消、输出上限。

### 6.2 建议迁移顺序

1. `base` / `contract` 的类型、错误、事件、配置 schema。
2. `config` + `provider` + 只读工具。
3. `state` 读路径；再写路径、迁移、checkpoint。
4. `agent` loop、审批、预算、取消、delegation。
5. `tools` 全量、`ext` MCP/插件/技能/Hook。
6. `safety` / `platform` 三平台。
7. `frontend` CLI/TUI/serve/ACP。
8. `studio-host` + Electron smoke。
9. 切流、删 Go、发布。

### 6.3 Shadow mode 规则

- 只对**只读或可重放**行为做双跑对比。
- 禁止把写文件、执行命令、调用真实模型的双跑落到真实 workspace。
- 使用 record/replay fixture；所有副作用在隔离临时目录。
- 对比事件流、工具调用、错误码、最终状态，不只对比 stdout。
- 差异必须归因为“reference 行为”“Rust bug”“有意变更”三类，并记录。

### 6.4 数据兼容

- 先只读打开 reference 生成的 SQLite 副本，验证 schema/查询。
- 再实现写路径；每次迁移必须幂等、可回滚。
- 不重命名表/列；如需新列，使用增量迁移并保留旧数据。
- 会话 ID、时间戳、排序、分页语义保持兼容。
- 提供 `reasonix db check` / `db migrate --dry-run` 类诊断（不改变现有 CLI 行为）。

---

## 8. 验收矩阵

### 8.1 功能

- CLI：命令、子命令、flag、env、exit code、help 文本语义。
- 配置：全键、默认值、优先级、迁移、keyring、secret 引用。
- Provider：流式、工具调用增量、重试、超时、用量、错误分类。
- Agent：turn、审批、plan、delegation、compaction、memory、budget、cancel。
- Tools：目录、schema、审批、输出上限、截断/落盘、shell AST 安全。
- State：会话、历史、搜索、checkpoint/rewind、usage、并发 lease。
- Ext：MCP、插件、技能、Hook、主题、market。
- Frontends：TUI、serve、ACP、studio-host。
- Desktop：Electron 启动、窗口、托盘、偏好、更新、崩溃报告。

### 8.2 协议与数据

- ACP：官方 TypeScript SDK conformance。
- MCP：stdio 与 HTTP 参考实现互通。
- Extension v2：manifest、handshake、stream、UI、drain、receipt。
- HTTP：路由、方法、状态码、JSON schema、错误码。
- SSE：事件名、data、id、重连、Last-Event-ID、顺序。
- SQLite：schema、迁移、索引、排序、分页、时间戳。
- 配置：round-trip、unknown key、路径展开、环境变量。

### 8.3 平台

- Windows x64/arm64：conpty、路径、注册表、Job Object、WSL、keyring。
- macOS x64/arm64：seatbelt、签名、notarization、keychain、托盘。
- Linux x64/arm64：bwrap、seccomp（按 reference）、keyring、桌面通知。
- 换行、编码、长路径、UNC、符号链接、文件锁。

### 8.4 性能预算

在同一机器、同一 fixture 下对比 reference：

| 指标 | 预算 |
|---|---|
| 冷启动 `--version` | ≤ reference + 20% |
| host 空闲 RSS | ≤ reference |
| 发布二进制大小 | ≤ reference × 1.5 |
| SSE 端到端 p95 | ≤ reference + 20% |
| 10k 文件工作区扫描 | ≤ reference + 20% |
| 单 turn 非模型开销 | ≤ reference + 20% |
| 24h soak RSS 增长 | ≤ 5% |
| 1k 会话加载 | 无 O(n²)、无无界内存 |

性能优化必须由 profile 驱动；M3 前不做微优化。

### 8.5 安全

- `cargo audit` 无未处理漏洞；`cargo deny` 许可证/源通过。
- `unsafe` 清单审查；每个 block 有 `// SAFETY:`。
- Fuzz：配置、SSE、MCP frame、SQLite 迁移输入； nightly 至少 10 分钟/目标。
- 安全 corpus：路径穿越、symlink、zip slip、SSRF、重定向、shell 注入、命令替换、环境泄漏。
- 日志/错误/崩溃报告/trace 不含 secret；有自动化 redaction 测试。
- 沙箱/权限行为与 reference 对齐；任何放宽必须有 ADR 和安全负责人批准。

### 8.6 质量门禁

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo nextest run --workspace --all-features
cargo test --workspace --doc
cargo deny check
cargo audit
cargo xtask parity --check
cargo xtask golden --check
cargo xtask routes --check
cargo xtask tools --check
cargo xtask deps --check
```

覆盖率目标：

- workspace 总体 ≥ 70%。
- `reasonix-safety`、`reasonix-state`、`reasonix-agent` ≥ 80%。
- 关键协议/迁移/安全路径必须有显式测试，不以覆盖率替代。

### 8.7 交付

- 六目标构建产物 + checksum。
- 安装包、签名、notarization、自动更新。
- SBOM、第三方许可证清单。
- 迁移指南、回滚指南、已知差异列表。
- 性能报告、安全审查记录、parity 账本。

---

## 9. 测试策略

### 9.1 测试层次

- **单元测试**：每个 crate，覆盖纯逻辑、错误映射、schema。
- **契约测试**：serde round-trip、JSON schema、HTTP/SSE/ACP/MCP frame。
- **Golden 测试**：reference 录制，规范化对比。
- **集成测试**：启动 host，真实 HTTP 客户端，隔离 home。
- **E2E**：Electron smoke、ACP SDK、MCP 服务器、三平台。
- **属性测试**：配置优先级、路径规范化、shell 解析、事件排序。
- **Fuzz**：配置、协议帧、迁移输入、URL/路径。
- **并发测试**：取消、超时、并发 turn、SQLite 写锁、lease。
- **性能测试**：criterion + hyperfine + 自定义 SSE harness。

### 9.2 测试环境

- 每个测试使用独立临时 `REASONIX_HOME`，不读写真实用户目录。
- 模型调用使用 mock provider 或 record/replay；CI 不持有真实 key。
- 时间、UUID、端口、路径可注入/规范化。
- 测试失败时保留最小 fixture，不保留用户数据。
- 跨平台测试使用 CI matrix；本地至少跑当前平台。

### 9.3 测试禁止事项

- 禁止依赖执行顺序、真实网络、真实模型、真实用户配置。
- 禁止用 `sleep` 等异步完成；使用条件变量/事件。
- 禁止为了让 golden 通过而手工改 expected；必须解释差异。
- 禁止只测 happy path；取消、超时、拒绝、损坏输入必须有测试。

---

## 10. Rust 工程规范

### 10.1 工具链与静态检查

- `rust-toolchain.toml` 固定 stable 版本。
- `rustfmt.toml`、`clippy.toml` 提交到仓库。
- `#![forbid(unsafe_code)]` 作为默认；例外 crate 显式 `#![deny(unsafe_op_in_unsafe_fn)]`。
- `cargo clippy -- -D warnings`；不允许用 `#[allow]` 大面积压制，必须逐条说明。
- `cargo deny` 检查许可证、重复依赖、禁用源。
- `cargo machete` 检查未使用依赖。

### 10.2 错误处理

- 库 crate 用 `thiserror`；二进制入口可用 `anyhow` 包装。
- 领域错误必须有 typed code；跨进程错误用 dotted code。
- 禁止靠 `err.to_string().contains(...)` 分类。
- 用户可见错误保留 reference 的语义与退出码；内部错误附加上下文。
- 不在错误中泄露 secret 或完整文件内容。

### 10.3 异步与并发

- Tokio 多线程运行时；阻塞操作 `spawn_blocking`。
- 所有外部调用有超时；所有流有取消路径。
- 使用 `CancellationToken` / `watch` / `mpsc`，避免裸 `JoinHandle` 泄漏。
- channel 必须有界；输出、事件、日志都要限流/截断。
- 共享状态优先 actor 或 `RwLock` 分片；记录锁顺序，避免死锁。
- 关键并发逻辑用 `loom` 或压力测试验证。

### 10.4 序列化与协议

- serde 显式 `rename` / `rename_all`，不依赖默认命名。
- 配置保留未知字段（除非 reference 明确拒绝）。
- 事件/协议类型加版本字段；新增可选字段保持兼容。
- 禁止用 `serde_json::Value` 承载核心业务状态；边界层可以。
- 对 JSON schema 使用 `schemars` 生成/校验，与 reference schema 对比。

### 10.5 日志与可观测性

- 使用 `tracing`，结构化字段；库不直接 `println!`。
- 日志级别：error/warn/info/debug/trace；默认不输出 debug。
- 敏感字段 redaction；token、cookie、API key、文件内容不落日志。
- 崩溃报告、trace、span 命名与 reference 对齐，便于对比。

### 10.6 依赖政策

- 优先纯 Rust、维护活跃、许可证兼容（MIT/Apache-2.0）。
- 禁止 OpenSSL；TLS 用 rustls。
- 网络、加密、解析类依赖需要安全审查。
- 新依赖必须在 PR 说明用途、替代方案、体积/编译影响。
- 不引入与现有 crate 功能重复的库。

---

## 11. Agent 工作流

### 11.1 每次任务的标准步骤

1. 阅读本文件、`docs/parity/LEDGER.md` 和相关 reference 包。
2. 选择一个可独立验收的 ledger 行/垂直切片，不要同时改多个层。
3. 如涉及契约、依赖方向、错误模型、unsafe，先写 ADR。
4. 先写测试或 golden case，再写实现。
5. 实现最小可用版本，保持与 reference 行为一致。
6. 运行第 8.6 节门禁；修复所有失败。
7. 更新 ledger、文档、CHANGELOG（如有）。
8. 提交时附：改了什么、为什么、如何验证、还有什么未覆盖。

### 11.2 PR 规则

- 一个 PR 只做一个主题；目标 < 600 行变更，超过则拆分。
- 不混入无关格式化、重命名、依赖升级。
- 不提交生成的 `target/`、大 fixture、真实日志。
- 不修改 `reference/`。
- 不删除/跳过失败测试来“修绿”。
- 不引入新的用户可见行为而不更新协议/文档。
- 不把 `todo!()`、`unimplemented!()` 留在可达路径；未实现必须由 feature flag 隔离并记录 ledger。

### 11.3 行为判断顺序

当不确定时，按以下优先级：

1. reference 的实际运行结果（用隔离环境复现）。
2. reference 的测试与 golden 数据。
3. reference 的源码与 docs。
4. 本文件的架构规则。
5. 自己的推断（必须写 ADR/注释，不能默默决定）。

## 12. 风险登记

| 风险 | 影响 | 缓解 |
|---|---|---|
| Async 取消语义与 Go 不一致 | 流中断、任务泄漏、卡死 | 先做 cancellation POC；每个流有超时/取消测试；soak |
| 共享状态借用检查 | 进度停滞、过度 `Arc<Mutex>` | actor/channel 设计；锁顺序文档；压力测试 |
| 平台 API 差异 | Windows/macOS 功能缺失 | M0 先验证 conpty/seatbelt/bwrap/keyring；CI matrix |
| 安全不变量遗漏 | 漏洞 | 安全 corpus、fuzz、威胁模型、专人 review |
| SQLite 迁移不兼容 | 用户数据损坏 | 只读副本测试、幂等迁移、备份/回滚 |
| 协议漂移 | 前端/插件不兼容 | 冻结 inventory、conformance、golden |
| 测试重写被低估 | 进度超支 | 行为契约优先；不逐行翻译；自动化 golden |
| Electron/Tauri 差异 | 桌面端回归 | 先保留 Electron；Tauri 延后 |
| 团队 Rust 经验不足 | 质量/进度风险 | 结对、ADR、clippy、POC、代码 review |
| 范围蔓延 | 无法交付 | 非目标清单；ledger 外需求拒绝 |
| 发布/签名复杂 | 无法分发 | 复用现有流水线；尽早做六目标 dry run |

---

## 14. 术语

- **Reference**：`reference/DeepSeek-Reasonix-studio`，只读 Go 实现，行为 oracle。
- **Kernel**：`reasonix` / `reasonix-studio-host` 链接的业务内核，不含 React/Electron/Workers。
- **Parity**：同样的输入、配置、数据下，用户可见行为与 reference 一致。
- **Golden**：从 reference 录制的输入/输出对，用于回归对比。
- **Sidecar**：迁移期临时运行的 Go 进程，最终必须删除。
- **Cutover**：生产默认切到 Rust，Go 不再参与运行时。
- **Break**：有意不兼容的行为，必须有 ADR 和用户影响说明。

---

> **最终提醒**：这次重写的难点不是“把 Go 语法翻译成 Rust”，而是 **35.8 万行 Go 产品代码背后的协议、状态机、平台差异和安全不变量**。  
> 每次只推进一个可验收的垂直切片；先证明行为一致，再谈性能与优雅。
> 自主推进到完成，全部完成之前不要停下来

---
其他，你可以调用子代理，但是每次只能调用一次，而且调用之后你需要暂停工作并且等他回来（意思是只能单并发）
需要什么自己下载，网络原因请走镜像（https://v4.gh-proxy.com/{github链接}）