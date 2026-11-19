# Go 项目画像（go-profile）

> 波次 17（2026-11-20）迁移窗口的侦察产物。权威契约在 `docs/contracts/*.md`（M1 冻结），
> 本文件只做画像汇总与规模计量；冲突时以冻结契约与 Go 源为准。

## 1. 项目用途

Reasonix —— AI 编码 agent 的桌面产品。三进程拓扑：

- **内核（本迁移对象）**：Go 包 `internal/**` + 交付二进制 `cmd/reasonix-studio-host`
  （Studio 的内核 host：loopback HTTP/SSE 后端）与 `cmd/reasonix`（CLI）。
  职责：会话存储（JSONL 事件日志 + SQLite 投影）、provider 消息循环、工具执行、
  审批/沙箱安全面、serve HTTP/SSE 前端、TUI/CLI/ACP 前端。
- **壳**：Tauri 2（`desktop-tauri`，Rust），spawn 内核为 sidecar，凭据 cookie 注入。
- **前端**：Vue 3（`desktop-tauri/src`），唯一线缆面 `port/hub.ts`。

## 2. Go 版本与工具链

`go 1.26.7`（go.mod），toolchain `go1.26.11`；golangci-lint v2.12.2（`.golangci-version`）。

## 3. 第三方依赖（go.mod，按用途）

| 用途 | 依赖 | Rust 对应（本波次涉及） |
|---|---|---|
| HTTP 服务 | `golang.org/x/net`、`gorilla/websocket` | axum 0.8 + tokio（本波次预置） |
| 存储 | `modernc.org/sqlite`（三个可重建投影库） | 本波次不触（Go 持有） |
| 配置 | `BurntSushi/toml`、`joho/godotenv` | `toml` 0.8（本波次预置） |
| 随机 | `crypto/rand`（stdlib） | `getrandom`（已在工作区） |
| 终端/TUI | charm.land/bubbletea、lipgloss 等 | 本波次不触 |
| 树解析 | tree-sitter 系列 | 本波次不触 |
| schema | `santhosh-tekuri/jsonschema` | 本波次不触 |
| 其余 | sftp/keyring/qr/goldmark/starlark… | 本波次不触 |

## 4. 包规模（非测试 Go 行数，实测）

| Go 包 | 非测试行数 | Rust crate | 现状 |
|---|---|---|---|
| internal/state | 47,839 | reasonix-state | **done**（22,747 行，账本 SESS 大半 done） |
| internal/contract | 47,045 | reasonix-contract | 部分done（eventwire manifest、pricing、PMSG 20/20 done；config/settings 面 todo） |
| internal/ext | 54,884 | reasonix-ext | skeleton（本波次不迁） |
| internal/frontend | 77,856 | serve/cli/tui/acp | serve 30,677 / cli 19,425 / tui 11,108 / acp 8,296；**本波次迁 serve 的门+帧+读侧路由子集** |
| internal/platform | 45,235 | reasonix-platform | skeleton（本波次不迁） |
| internal/session | 35,816 | reasonix-session | skeleton（**本波次迁 hub 读侧 + Controller trait 冻结**） |
| internal/assembly | 9,782 | reasonix-assembly | skeleton（本波次不迁） |
| internal/safety | 16,876 | reasonix-safety | skeleton（本波次不迁） |
| internal/tools | 20,009 | reasonix-tools | skeleton（本波次不迁） |
| internal/runtime | 41,303 | reasonix-agent | skeleton（本波次不迁） |
| internal/model | 8,994 | reasonix-provider | skeleton（PMSG 类型已在 contract 完成；本波次不迁客户端） |
| internal/base | 14,736 | reasonix-core | 部分 done（fileutil/filelock/fileencoding done；其余 todo） |
| cmd/reasonix-studio-host | ~2,234 | apps/reasonix-studio-host | stub exit 2 → **本波次实现启动面** |
| cmd/ 其余 | CLI/工具/评测台 | apps/ + tools/ | 本波次不迁 |

测试规模：internal+cmd 测试文件 34,742 行；serve 包本波次相关测试
broadcaster_test.go(191) + eventstream_test.go(266) 为 port 的直接 oracle。

## 5. HTTP 接口清单（本波次相关子集）

全量 420 路由冻结于 `docs/contracts/http.md` §1。本波次实现的路由与形状：

| 方法/路径 | 形状要点 | 契约 |
|---|---|---|
| GET `/runtimes` | runtime 列表（空 hub → `[]`） | http.md §1.2 |
| POST `/runtimes` | 打开 runtime（本波次读侧：恒 409/拒绝？否——见 profile-host：未实现写侧，返回 refusal） | http.md §1.2 |
| GET `/rt/{id}/…` | runtime 前缀路由，未知 id → 404 `request.not_found` `{kind:"runtime"}` | http.md §0 |
| GET `/tree` | 工作区树（workspaces + sessions） | http.md §1.2 |
| GET `/sessions` | `{name, path, title?, turns?, current?, unread?, modified}` 数组；空/不可读目录 → `[]` | http.md §7 |
| GET `/history/page` | 会话历史分页（state 读模型之上） | http.md §1.1 |
| GET `/events` | SSE（帧格式/keepalive/watermark/Last-Event-ID 恢复） | http.md §5 |
| GET `/events/replay` | `{frames: [], complete: bool, watermark: N}` | http.md §5 |
| GET `/` | routeDefault：无 runtime 且已认证 → 503 `hub.no_runtime_open` | http.md §0 |
| GET `/status` | 会话级 status（无 runtime 时经 routeDefault 不可达） | http.md §7 |
| 门与信封 | loopback gate（403 dotted 三码）、Reason 信封 `{code,error,params}`、csrfGuard、mutationGate、auth none/token | http.md §2/§4 |

## 6. 数据模型（本波次触达）

- 会话目录布局与 JSONL 事件日志：Rust `crates/state` **已完成**（SESS-002..042 大半 done），
  本波次只消费其公开读模型（listing/order、range reads 若可用）。
- `serve-workspaces.json`：`{"paths":[...],"launch":...}`，读写归 hub（Go
  `internal/state/workspacelist/list.go` 341 行 + `internal/contract/config/storage_roots.go`）。
- 无业务 SQL 触碰（三个 SQLite 库是可重建投影，Go 持有）。

## 7. 配置项清单（本波次触达子集）

Go `internal/contract/config`（29,508 非测试行，全部 CONFIG 账本行 35 条）。
本波次只迁 host 启动所需：顶层加载/分层（用户层→项目层）、`serve` 段
（AuthMode/Token/PasswordHash/BehindProxy，供 HOST-018 清零）、desktop 键
（DesktopTelemetry/DesktopMetrics/DesktopTray）、`AutoWorkspaceDir`、顶层 `language`。
其余键面（providers/presets/memory/hooks/...）留待后续波次，见 backlog。

## 8. 并发与生命周期关键点（Go 侧现状）

- Broadcaster：每订阅者有界队列（live 4MiB / replay 8MiB），Emit 永不阻塞、
  序号 transport-scoped、ResetSession 保留计数丢弃尾巴（sse.md §3）。
- eventstream：keepalive 15s（comment + watermark 帧）、订阅五模式、
  订阅与回放的一次握手串行化。
- hub：`bindMu` 串行化会话路径/控制器代际变更；一个会话文件一个 runtime。
- host：stdin 租约（pipe/socket 才算），EOF = 优雅结束 exit 0；
  act 行互斥串行；10s socket drain。
- Rust 侧对应纪律：tokio 有界 mpsc / watch、锁不跨 await（见 optimization-plan L0）。

## 9. 外部集成（本波次触达）

无出站网络。provider/钱包/更新/遥测/市场等出站面全部留待后续波次。

## 10. 启动顺序与依赖图（host）

`信号→租约→FindPage→assemble→hub→bind(loopback+gate)→provider-setup enable→announce(握手)→serve(优雅排空)→hub.Shutdown`
（host.md §1.7 的 12 相位日志顺序 `page, config-upgrade, config, sinks, workspace, runtime, hub, adopt, background, bind, provider-setup, announce`）。
本波次实现其中的真实可执行子集；provider-setup/adopt/background 等依赖后续 crate 的相位
记录日志行但对应机器为空实现并明确标注（不伪造成功）。

## 11. 已有测试与 benchmark 清单（本波次相关）

| Go 测试 | 用途 | Rust 对应 |
|---|---|---|
| serve/broadcaster_test.go (191L) | 编号/掉帧/回放/水印 | crates/serve 单测 |
| serve/eventstream_test.go (266L) | SSE 帧/恢复/订阅模式 | crates/serve 单测 |
| serve/hub_projects_benchmark_test.go | 树构建 Benchmark | crates/session criterion 基准 |
| serve/history_page_test.go（Benchmark） | 历史分页 Benchmark | crates/serve criterion 基准 |
| host 系列（host.md §内列名） | 租约/握手/act/凭据 | apps/reasonix-studio-host 集成测试 |
| state crate golden 测试 | 读模型 oracle | 直接复用（已在 crates/state） |
