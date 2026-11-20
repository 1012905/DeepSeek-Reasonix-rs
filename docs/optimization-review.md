# 优化审查（波次 17）

> 审查人：Agent-Opt-Review（只读）。审查日期：2026-11-20。分支 `rust-migration`
>（HEAD `d76426b83`）。本文件只记录审查结论，不改任何代码、账本、契约或配置。

## 审查范围与方法

**通读范围（本波次落盘的全部 Rust 代码，约 10.2k 行）**：

- `crates/config/src/`：lib/load/model/serve/desktop/language/paths（含单测）
- `crates/session/src/`：lib/hub/model/tree/listing/workspacelist/title/paths/controller（含单测）
- `crates/serve/src/`：lib/gate/auth/refuse/listen/page/broadcaster/eventstream/router/boot（含单测）
- `crates/assembly/src/`：host.rs（含 lib.rs/boot.rs 对照）
- `apps/reasonix-studio-host/src/`：main/args/credential/handshake/acts/phases/lastline/lease；`tests/cli.rs`
- 基准：`crates/serve/benches/{gate,emit,frames}.rs`、`crates/session/benches/tree.rs`、`crates/config/benches/{load,direct,merge}.rs`
- golden：`crates/serve/fixtures/golden/manifest.json` + `crates/serve/tests/golden_replay.rs`

**对照材料（只读）**：`docs/contract.md`（切片 S1-S4、§3 裁剪、§4 冻结签名）、
`docs/optimization-plan.md`（OPT-001..020）、四份冻结契约（http/sse/host/config.md）、
Go 源（`internal/frontend/serve/{loopback,host_guard,launch_token,fail,listen,page,
broadcaster,eventstream,sessions_list,refuse_common,auth,session_order,hub,hub_tree}.go`、
`cmd/reasonix-studio-host/main.go`、`cmd/reasonix-studio-host/{startup,launchroot}.go`、
`internal/state/workspacelist/list.go`）、`OPEN_QUESTIONS.md`、`docs/parity/{LEDGER,BREAKS}.md`、
`docs/migration-state.md`、四份 profile 任务书、`docs/optimization-backlog.md`（L3 禁区清单）。

**方法**：逐文件通读 + 与 Go 源逐行为对照（oracle 优先级：行为>测试>源>文档）+
并发纪律专项（锁内 await / 有界性 / 任务收口 / emit 阻塞）+ 错误处理专项
（非测试路径 unwrap/expect/panic）+ L1 证据复跑。本机执行了：

- `cargo test -p reasonix-session -p reasonix-serve -p reasonix-assembly -p reasonix-studio-host`
  → 全绿（session 51、serve lib 82 + golden 3、assembly 10、host 14+4）。
- `cargo clippy -p …（五个 crate）--all-targets` → 0 警告。
- 基准复跑：`gate_check_zero_alloc ≈ 44.8ns` vs `gate_check_naive_rebuilt ≈ 566ns`；
  `frame_render_single_buffer ≈ 124.9ns` vs `frame_render_direct_allocs ≈ 525.4ns`；
  `tree_500_sessions/optimized ≈ 46.1ms`（literal 对照同场可跑）。与 migration-state.md
  记录的 40/266ns、122/463ns、28-34ms 同量级，**可复现**。
- `crates/config` 的 14 个测试与 `benches/load.rs` 在本机失败——全部败于
  `TempDir::new()` 触碰被策略禁写的系统 Temp（详见发现 N-1），非逻辑失败。

## 发现

无 BLOCKER。3 条 MAJOR、8 条 MINOR、7 条 NOTE。

### MAJOR

**M-1　`GET /sessions` 注册在 hub 层，无 runtime 时答 200 `[]`，Go 答 503 `hub.no_runtime_open`**

- 位置：`crates/serve/src/router.rs:109-112`（注册）、`crates/serve/src/router.rs:268-277`
  （`get_sessions`）、`crates/serve/src/router.rs:686-700`（钉住该行为的测试）。
- 问题：Go 的 hub mux（hub.go:540-553，http.md §1.2 desktop-host 面）**不注册** `/sessions`；
  该路径落 `routeDefault`——无 runtime 时答 503 `hub.no_runtime_open`，有 runtime 时
  转发到第一个 pane 自己的 `s.sessions`（current/unread 按该 pane 计算）。
  Rust 在 hub 层注册并直接用「第一个 runtime 的 root」推导目录，无 runtime 时
  依 `session_dir_for(state_root, "")` 回退全局 `sessions/` 目录，答 200 `[]`。
- 证据：Go `hub.go` Handler 注册表无 `/sessions` 行（本审查已核对 540-553 行原文）；
  Rust 测试 `tree_and_sessions_answer_arrays_never_null` 把 200 `[]` 钉成了期望行为。
  契约锚点：http.md §1.2（74 行 desktop 面清单无 /sessions）、§0 routeDefault。
- 影响：无 runtime 的 hub 上一个可探测端点从 503 变 200；有 runtime 后 pane 自身的
  `current` 语义（canonical path 比对）也不会到达该端点。
- 处置建议（无 L3）：把 `/sessions` 从 hub 路由表移除（落回 routeDefault 的 503，
  与 Go 拓扑一致），或至少在 `get_sessions` 内无 runtime 时改答
  `no_runtime_open()`；同步修正钉住 200 的测试。属实现层改动，不动信封/码表。

**M-2　host 启动把两个项目配置层都合并，Go 是「根 `reasonix.toml` 存在即只用它，不合并」**

- 位置：`crates/serve/src/boot.rs:12-19`（`load_config` 把 `cwd/reasonix.toml` 与
  `cwd/.reasonix/reasonix.toml` 两个路径都塞进分层列表），
  `crates/config/src/load.rs:41-63`（每层都 `apply_layer`，后层按键覆盖）。
- 问题：config.md §0 与 §12.6（CONFIG_PATHS）冻结：项目配置是
  `./reasonix.toml` **回退**到 `.reasonix/reasonix.toml`，「Root project file wins when
  both exist (NOT merged); only absence (not malformed…) permits fallback」（paths.md
  §2 / PATHS-006 同义）。两个文件同时存在时，Go 只读根文件；Rust 会把两者都生效
  （`.reasonix` 侧按键胜出），任何两文件共存的工作区里生效配置都可能不同。
- 证据：`boot.rs` 组装序列 `once(user_config_path()).chain([root/reasonix.toml,
  .reasonix/reasonix.toml])`；`Config::load` 对每一层无条件 merge；
  契约原文见 config.md §0、§12.6。
- 处置建议：在 `load_config` 里按 PATHS-006 语义做存在性探测——根文件存在则只取根
  文件，否则才落 `.reasonix/reasonix.toml`（探测需遵循「仅 IsNotExist 许回退」）。
  不改 `Config::load` 签名与 API 形状。

**M-3　`-instance-id` 在 Windows 上哈希入参被改写成 `/` 分隔，与 Go 的 `filepath.Clean` 值不同（HOST-021 已翻 done）**

- 位置：`crates/assembly/src/host.rs:195-215`（`clean_path` 按两种分隔符切开、
  一律以 `/` 重拼）、`crates/assembly/src/host.rs:154-173`（`instance_id_for`
  对 `Sha256::digest(clean_path(&root))` 取前 8 字节）。
- 问题：host.md §5.1 冻结身份为 `sha256(filepath.Clean(root))[:8]`。Go 的
  `filepath.Clean` 在 Windows 产出反斜杠路径（如 `C:\Users\x`），而 Rust
  `clean_path("C:\Users\x")` 产出 `C:/Users/x`——同一数据 home 的哈希输入不同，
  产出的 `io.reasonix.studio.<16hex>` 与 Go host 对同一 home 的答案不一致。
  该值是壳做 profile 目录与单实例锁 scoping 的键（host.md §7.4）：Go→Rust
  切换时实例身份整体换值（窗口状态/锁作用域迁移一次），且当前工作树里 Go host
  仍是 Tauri 壳的 sidecar（OPEN_QUESTIONS §1），两套二进制并存期间同一 home
  会得到两个身份。
- 证据：`clean_path` 的 `parts.join("/")`（host.rs:213）与
  `let absolute = path.starts_with('/')`（host.rs:196，对 `C:\…` 形状判为相对）；
  现有测试 `the_id_is_prefix_plus_eight_bytes_hex_and_stable_per_home` 用同一
  Rust 函数自证，钉不住与 Go 的字节一致。host.md §5.1、§8 注 1（要求与 Go
  逐字节对拍的精神）。
- 处置建议：哈希入参改用保持 OS 分隔符的 Clean（复用 `crates/session/src/paths.rs`
  已有的 `clean`（其 Windows 分支保 `C:\` 形状，且该实现本就锚定 PATHS-001/003
  收编进 config），或在 canonicalize 分支直接对 `canonical_existing` 的结果做
  尾随分隔符剥离后原样哈希。补一条与 Go `filepath.Clean` 期望字面量的对照测试。

### MINOR

**m-1　`pick_launch_workspace` 丢掉 launch 项目优先（host.md §6.1/HOST-026）**

- 位置：`crates/assembly/src/host.rs:81-85`（用 `workspacelist::workspaces()` 取
  remembered 序列）。
- 问题：host.md §6.1「the launch project wins, then the remembered order」；session
  crate 已实现 `launch_workspaces()`（`crates/session/src/workspacelist.rs:90-105`，
  profile-session 任务书第 1 条也点名），assembly 却用了不提升 launch 的
  `workspaces()`。HOST-025/026 尚未翻 done，故不算翻行偏差，但机器一旦接线即埋雷。
- 处置：改用 `launch_workspaces`（已有实现），并在接线 HOST-025 前完成。

**m-2　serve 侧 token 生成在 CSPRNG 失败时 panic（Go 是 error→exit 1）**

- 位置：`crates/serve/src/auth.rs:170-175`（`generate_token` 内
  `unwrap_or_else(|_| panic!(…))`）。
- 问题：Go `auth.go generateToken` 出错由调用方回错误、进程以 exit 1 收场
  （host.md §1.3）。Rust 的 serve 生成路径把失败换成 panic——属「panic 路径在
  非测试代码」，且绕过 CatchPanicLayer 之外（构造期）直接打穿进程。概率极低
  （seccomp/沙箱场景），host 主路径用的是返回 `Result` 的
  `apps/reasonix-studio-host/src/credential.rs`（正确），仅 serve-CLI 形状可达。
- 处置：`generate_token` 改为返回 `Result<String, getrandom::Error>`，或由调用方
  `AuthGate::none` 显式降级为 invalid-门 + 日志；不建议保留裸 panic。

**m-3　排空期不区分空闲 keep-alive 连接，浏览器开着页面时退出恒吃满 10s**

- 位置：`crates/serve/src/listen.rs:57-68`（drain 循环等 `live` 计数归零，20ms 轮询）、
  `crates/serve/src/listen.rs:99-131`（连接级 120s idle 看门狗）。
- 问题：Go `runGracefulListener`（listen.go:52-81）在 ctx 结束时调 `srv.Shutdown`，
  Shutdown **立即关闭空闲 keep-alive 连接**、只等有在途请求的连接直到 10s 预算；
  Rust 的 drain 以「连接数」为在途判据，空闲 keep-alive 连接也要等满 10s 预算
  才放行进程退出（端口本身随 accept loop 结束即释放，HOST-006 的 exit 0 不受影响，
  只是 quit→进程退出的尾延迟被拉长到 10s）。
- 处置：shutdown 时先向所有连接任务广播一个「立即关空闲」信号（例如在
  `serve_connection` 的 select 中加 watch 分支：无在途请求即 break），或 drain 判据
  改为在途请求数。属实现层，不改监听 API。

**m-4　OPT-008 的信号路径无测试（只测了租约 EOF）**

- 位置：`apps/reasonix-studio-host/tests/cli.rs`（仅 `a_launch_whose_lease_ends_drains_cleanly`）；
  机制在 `apps/reasonix-studio-host/src/main.rs:116-138`（ctrl_c + SIGTERM 双任务
  只 `cancel()`，主循环 `cancelled_owned()` 收口——实现正确且幂等）。
- 问题：optimization-plan OPT-008 证据要求「EOF 与 SIGTERM 两路都收敛 exit 0」。
  SIGTERM 一路没有集成测试（Windows 上确难注入；unix cfg 下可用子进程发信号）。
- 处置：补一条 unix-only 集成测试（spawn + `kill -TERM` + 期待 exit 0），或把
  该缺口记入账本 HOST-006 行的注记。

**m-5　OPT-002 的证据方法未按计划落盘（缺「断开后任务数回落」断言）**

- 位置：机制 `crates/serve/src/eventstream.rs:154-183`（每连接一个 task，
  `select!` 于帧/keepalive/`tx.closed()`，断连即收口；`Subscription` drop →
  `unsubscribe`，`crates/serve/src/broadcaster.rs:546-559`）。
- 问题：计划要求「client 断开后任务数回落」的直接断言；现有最接近的是
  `unsubscribe_drops_the_subscriber_and_emit_survives`（订阅移除）与
  `events_stream_opens_with_connected_then_numbers_frames`（oneshot 全程），
  均非「真断连→任务收敛」。机制审查未发现泄漏，但证据不完整。
- 处置：补一条真实 TCP 断连测试（连接→断开→断言 `subscriber_count` 回 0），
  或在账本 OPT-002 行注记证据为句柄 drop 路径推演。

**m-6　OPT-009 的「并发 64 act」测试缺失**

- 位置：`apps/reasonix-studio-host/src/acts.rs:39-52`（`std::sync::Mutex` 短临界区、
  单次 `write_all(line)`——实现完全符合计划与伪优化禁区要求）；测试
  `acts.rs:77-86` 为顺序两行。
- 处置：补多线程 64 act 并发写的成行测试（`ShellOwner` 本就可并发调用）。

**m-7　OPT-015 的证据（复杂度分析 + 未量化标注）未见落盘**

- 位置：`apps/reasonix-studio-host/src/phases.rs`（Instant + 常量名相位表，实现
  显然 O(1)/相位、无格式化开销问题）。
- 问题：计划要求「复杂度分析 + 未量化标注」作为该 L1 的证据形态；migration-state
  与 profile 文档均未记录这段分析。实现无可挑剔，属证据留痕缺失。
- 处置：在账本 HOST-004 行或 profile-host.md 补两行注记（12 相位 O(1)，启动期
  一次性行，不设基准）。

**m-8　config 测试与 config 基准依赖系统 Temp 可写，本机策略下 14 测试 + OPT-016 基准不可复现**

- 位置：`crates/config/src/load.rs`（各测试直接 `TempDir::new()`，如 load.rs:227）、
  `crates/config/benches/load.rs:54`。
- 问题：本波次其余 crate 均已改用 `target/test-tmp`（session/serve/assembly 的
  `test_support::tempdir`，lib.rs 注释明确说明原因），config 漏改。本机
  `%TEMP%` 被拒写时：`cargo test -p reasonix-config` 14 失败（全部
  PermissionDenied）、`cargo bench -p reasonix-config` 直接 panic——
  migration-state「46 套件全绿」的可复现性声明在这类环境不成立。
- 处置：与其它 crate 统一改用 target 下的临时目录助手；不影响正确性结论。

### NOTE

**n-1　refuse.rs 遗留死代码**：`crates/serve/src/refuse.rs:136-141`（`json_params`）与
`327-330`（`_params_shape_check`，`#[allow(dead_code)]`）从未被调用。建议下波次清理。

**n-2　路径语言暂存两份实现**：`${VAR}` 展开与 `filepath.Clean` 在
`crates/config/src/paths.rs`（Component 版 `clean_path` + `expand_vars_lookup`）与
`crates/session/src/paths.rs`（字符串版 `clean` + `expand_vars_with`）各有一份，
语义目前一致但已是第三种路径语言的风险面。两份文件各自注明待 PATHS-001/003
收编——与 OPT-007「不重写第三份」精神一致，维持现状可接受，收编时必须二合一。

**n-3　routeDefault 页面分支缺条件请求/Range**：`crates/serve/src/router.rs:304-325`
直接 `fs::read(index.html)` 恒 200；Go 用 `http.ServeFileFS`（支持 If-Modified-Since/
Range）。冻结面（routeDefault 的 503/200 分叉）不受影响，属 http.ServeFileFS
能力子集；建议在 ROUTE-003 行注记。

**n-4　withPage 文件服务缺条件请求**：`crates/serve/src/page.rs:139-153` 同 n-3
（Go `http.FileServer` 支持 Range/If-Modified-Since）。账本 ROUTE-006 已注记
「304/If-Modified-Since outstanding」，不重复计为漂移。

**n-5　broadcaster 的 Go 侧边状态未随 Emit 迁移**：display_currency 重绑、workMs
累计、usage/cost ledger、coldStart（Go broadcaster.go:145-165）在
`crates/serve/src/broadcaster.rs` 的 `emit` 中不存在。对应机器（CostQuoteSink/
`/status` 的 sessionCostQuote）在 contract.md §3 裁剪清单与 ROUTE-030 todo 内，
属裁剪而非漏迁；`reset_session` 也因此只清 replay/initial-notices（正确）。
待 B-005/SSE-011 波次补齐时一次接上。

**n-6　stream_gap/stream_watermark 以 `Event{kind:…}` 建模**：sse.md §9.4 建议
「作为传输帧而非事件 kind 建模」；Rust 复用 wire 信封类型（`crates/serve/src/
broadcaster.rs:474-486`），线上字节与 Go 完全一致（`watermark_json` 手拼、
gap 走 Event 序列化，字段集一致）。语义等价，仅提一句与契约建议的形状差异。

**n-7　冻结面 API 暂无调用方**：`mount_runtime_routes`（router.rs:457-460）、
`emit_to`/`set_on_tree_changed`（broadcaster.rs:292-308/431-434）、`acts::ShellOwner`
（main.rs 标 `#[allow(dead_code)]`）在本波次无运行时调用方——均为 contract.md §4
冻结签名或后续波次锚位，保留合理；接线下一波次时注意别为「用上它」而制造调用。

**伪优化专项核查（无发现，列出核查面）**：全部 10k 行 `#![forbid(unsafe_code)]`；
无「为泛型而泛型」；无以 clone 掩盖所有权问题（`Bytes` 共享帧数据是真实共享而非
掩盖）；锁均为 `std` Mutex 且临界区内无 await（broadcaster 双锁嵌套、`Notify`
等待前先 `enable()` 防丢单，`crates/serve/src/broadcaster.rs:505-528` 是教科书式
写法）；所有队列有界（body 64KiB、回放 8MiB、每订阅者 4MiB、连接 mpsc 16、
act 单 write）；emit 全程无 await、不阻塞（基准与测试双向钉住）；
`constant_time_eq` 手写 XOR 折叠有长度无关测试钉住（gate.rs:260-269），无 unsafe。

## OPT 条目证据核对表

| OPT | 级别 | 证据状态 | 说明 |
|---|---|---|---|
| OPT-001 | L0 | 落盘 | 有界队列+逐级摘除实现正确；`emit_does_not_block_on_a_stalled_client`、`sheds_droppable_frames…` 钉住有界性与不阻塞 |
| OPT-002 | L0 | 部分落盘（存疑） | 收口机制正确（select!+tx.closed+Subscription drop）；计划要求的「断连后任务数回落」测试缺失（见 m-5） |
| OPT-003 | L0 | 落盘 | `CatchPanicLayer` + `a_panicking_handler_answers_500_text`（500 text/plain）+ `tracing::error!` panic 摘要 |
| OPT-004 | L0 | 落盘 | `oversized_body_refuses_as_bad_body` 同时覆盖撒谎 Content-Length 与 chunked 超限；拒绝码 `request.bad_body` |
| OPT-005 | L0 | 落盘 | 405/404 信封测试覆盖已注册路由全集（本波次仅 5 条 hub 路由 + 2 条 runtime 路由） |
| OPT-006 | L0 | 落盘 | 树/列表容忍语义照抄 Go（坏目录→warn+空、missing→missing 节点、TestTreeSendsAnEmptyList… 移植） |
| OPT-007 | L0 | 落盘 | canonical 键全部复用 `reasonix_state::canonical_session_path`；树键一致性两测（含 Windows 大小写折叠） |
| OPT-008 | L0 | 部分落盘 | CancellationToken 单一关闭源实现正确且幂等；EOF→exit 0 有集成测试，SIGTERM 路径无测试（m-4） |
| OPT-009 | L0 | 部分落盘 | std Mutex 短临界区 + 单 `write_all` 正确；「并发 64 act」测试缺失（m-6） |
| OPT-010 | L0 | 落盘 | 未知键透传/坏层整层丢弃/警告文本逐字/auth 归一化矩阵均有对照测试（覆盖本波次键面） |
| OPT-011 | L1 | 落盘且本机可复现 | `benches/gate.rs` 双形状对照；migration-state 记录 40/266ns，本机复测 44.8/566ns |
| OPT-012 | L1 | 落盘 | `benches/emit.rs` 四场景可复现；droppable 先行判定实现正确；基准数字未写入任何文档（仅可复现），Go 基线按计划标「未量化」 |
| OPT-013 | L1 | 落盘且本机可复现 | `benches/frames.rs` 单缓冲 vs 直译（记录 122/463ns，本机 124.9/525.4ns）；帧边界行为测试（首块恰为 `: connected\n\n`）钉住不跨帧攒写 |
| OPT-014 | L1 | 落盘且本机可复现 | `benches/tree.rs` 100 根 + 500 会话双形状（optimized/literal）；Go 基线 89.1ms 与口径差异注记在 migration-state |
| OPT-015 | L1 | 存疑（证据缺留痕） | 实现符合意图（O(1)、无格式化浪费）；计划的「复杂度分析+未量化标注」书面记录缺失（m-7） |
| OPT-016 | L1 | 落盘（存疑） | `benches/{load,direct,merge}.rs` 三文件落盘、策略对比可跑；结论（Value-merge 负收益）记录于 migration-state；但 (a) 方案与计划预设相反——属**有证据的方案否决**，允许；(b) 数字本身未存档；(c) 本机因 %TEMP% 受限不可复现（m-8） |
| OPT-017 | L2 | 落盘 | `serve::refuse` 单一模块：19 码常量表 + `RefuseError` 枚举 + `write_err` 按 fail.go 顺序；clippy 0；全部路由走统一信封 |
| OPT-018 | L2 | 落盘 | `session::model` 单一 DTO 集；serve 直接序列化同型，无第二份声明 |
| OPT-019 | L2 | 落盘 | `main` 只 `return ExitCode`（OPT-019 归一）；cli.rs 钉 2/1/0 三主路径；各启动失败子路径未逐一注入（可接受） |
| OPT-020 | L2 | 落盘 | `host_serve_section()` 类型化清零 + 四字段恒默认测试（crates/config/src/serve.rs:96-103、163-181） |

## 契约漂移清单

1. **（MAJOR，M-1）** `GET /sessions` 注册面：hub 层直答 200 `[]`；Go 拓扑下该路径
   不在 hub mux，无 runtime 答 503 `hub.no_runtime_open`（http.md §1.2/§0）。
2. **（MAJOR，M-2）** 项目配置双文件合并：`./reasonix.toml` 与 `.reasonix/reasonix.toml`
   共存时 Rust 双层生效，Go 只取根文件（config.md §0/§12.6、PATHS-006）。
3. **（MAJOR，M-3）** `-instance-id` Windows 哈希入参分隔符改写，同一 home 的
   16-hex 身份与 Go 不一致（host.md §5.1）。
4. **（MINOR，m-1）** 启动工作区回退未让 launch 项目优先（host.md §6.1）。
5. wildcard-bind 时 hostGuard 接口枚举缺失、失败闭合——**已由账本 ROUTE-012
   wip 注记**，不重复计。
6. 用户层损坏仅回默认、无 LKG 恢复——**CONFIG-006 todo 范围**，不重复计。
7. SSE `Emit` 的定价/货币/workMs 边状态未迁——**contract.md §3 裁剪面
   （B-005/SSE-011 todo）**，不重复计。
8. 租约分类窄化（terminal 判定）与 schedule-exec/strip-package-grants 的保守失败
   ——**已记录于 OPEN_QUESTIONS §7/§4**，不重复计。
9. FindPage 回退候选以 Go 源为准（`desktop-tauri/dist`）而非契约文档文本
   ——**已记录于 docs/parity/BREAKS.md BREAK-002**，不重复计。

除此之外未发现字段名、状态码、错误码、帧格式、订阅语义、相位顺序、退出码层面的
其它漂移：信封键序/换行（refuse.rs 测试）、握手字段序/尾换行（handshake.rs 测试）、
12 相位顺序、五订阅模式、gap/watermark 帧、keepalive 15s 双写、`/events/replay`
三键形状、`/rt/{id}` 404 `{kind,name}`、405/415/401/403 码表、csrf 二进制例外、
`/auth/token` 204+cookie、token 模式 302 清参、hostServeConfig 四字段清零、
acts 三词汇与 Linux-only relaunch、gate 四拒绝码（golden 与 Go 实录逐字节对拍）
均与契约一致。

## 结论

**需修正后放行。**

无 BLOCKER；骨架、并发纪律、伪优化禁区与 L1 证据主体合格（本机复跑了
session/serve/assembly/host 全部测试与三项基准，全部通过/可复现）。3 条 MAJOR
均为实现层可低成本修正的漂移，无一条需要 L3 变更或动冻结签名。

**需回原实现方修正的清单（放行前置条件）**：

1. **M-1**：`crates/serve/src/router.rs` 移除 hub 层 `/sessions` 注册（或无 runtime
   时答 503 `hub.no_runtime_open`），同步修正钉住 200 的测试
   （`tree_and_sessions_answer_arrays_never_null`）。
2. **M-2**：`crates/serve/src/boot.rs::load_config` 按存在性探测二选一项目层
   （根文件存在即不用 `.reasonix` 回退），补一个双文件共存的行为测试。
3. **M-3**：`crates/assembly/src/host.rs` 的 instance-id 哈希入参改用保持 OS
   分隔符的 Clean（复用/收编 `session::paths::clean`），并补 Go `filepath.Clean`
   字面量对照测试（含 `C:\…` 与 `\\?\` 剥离后形状）。

**建议随修正一并处理（不阻塞放行）**：m-1（launch 项目优先，一行改动）、
m-2（token panic 改 Result）、m-8（config 测试/基准换 target 临时目录）；
m-4/m-5/m-6/m-7 可先在账本对应行注记证据缺口，随各自机器波次补齐。

---

# 追加审查（波次 18：工具 ABI + 权限判定核）

> 审查人：主 Agent（复核口径同波次 17：Go 测试 oracle 对照 + 门实跑）。日期 2026-11-20。

## 范围

- `crates/tools/`（11 源文件 + 5 测试文件，63 测试）—— Agent-ToolAbi 产出
- `crates/safety/src/permission/`（12 文件，57 测试 + 8 锚位）—— Agent-Permission 产出

## 复核结论：放行（零 MAJOR/MINOR）

- **oracle 对照**：Go arguments_test 20 用例、invoke_test 全用例、TestParseRule 17 用例、
  registry/contract/provenance/write_fence/verdict/gate 测试表逐用例移植，测试名语义对应。
- **并发纪律**：registry 单锁内快照、锁外回调（AB-BA 死锁有回归测试
  `contract_entries_does_not_hold_registry_lock_across_tool_callbacks`）；permission 的
  Mutex 用 `PoisonError::into_inner` 中毒恢复（不 panic）；无跨 await 持锁。
- **错误身份**：Refusal{code}、BlockedError downcast、GateError thiserror——
  全树无错误字符串匹配（仓库 error-text 铁律），审查未发现违例。
- **诚实锚位**：mvdan AST 级 bash 分析以 `BashAnalysis` trait + fail-closed
  `StaticBashAnalysis` 表达，8 个 `#[ignore]` 锚位测试携带 Go oracle 表原文——
  不伪造只读/网络判定（sensitive 路径纪律）。
- **伪优化专项**：无 unsafe、无超计划行为改动；OPT-T001..003（L2）为结构收敛，
  无需基准。
- **主 Agent 三项决策（记录）**：① `BashAnalysis` trait + OnceLock 引擎 seam 为冻结面；
  ② origin 显式参数化（替代 Go context value）；③ CapabilitySet TypeId-downcast 认可。

## OPT 条目证据核对（波次 18 增量）

| OPT | 级别 | 状态 |
|---|---|---|
| OPT-T001 错误分类单点化 | L2 | 落盘（disabled→MCP disabled→held 同序测试） |
| OPT-T002 schema 快照一次化 | L2 | 落盘（schemas_canonicalizes_once_per_tool） |
| OPT-T003 duplicate-key 前置 | L2 | 落盘（unwrap_arguments_cases 三形态） |

## 备注

- permission 面（原计划表 OPT-001..020 之外）无既有热点条目；两 Agent 的迁移内
  自施优化（deny 规则免二次解析、正则改手写校验免 regex 依赖）均有测试对照，属 L2。
- 下一片解锁：bash AST 引擎（shellparse AnalyzeApprovalFeatures/SplitTopLevel/
  CompoundLeaves 等）装入 `set_bash_analysis` 即翻 SAFETY-008/009 的 8+2 锚位。
