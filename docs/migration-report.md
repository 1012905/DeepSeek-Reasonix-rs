# 迁移报告（波次 17：Rust 内核 host 启动切片）

> 分支 `rust-migration`（自 `feat/tauri-live-events` HEAD `0ade9fc5e`）。
> 任务口径：「Go 后端原地迁移为等价 Rust 内核 + 契约不变前提下的优化；验证通过后删除 Go」。
> 本仓库的实际状态是该迁移**已按 `docs/RUST_MIGRATION.md` 进行到 M2**（账本
> `docs/parity/LEDGER.md` 563 行、Go 内核约 42 万非测试行为 live 后端）。本波次按
> 任务编排把账本 M4（serve/host 切片）推进了一个完整可验证的增量；**Go 源未删除
> 未归档**（删除条件 = 账本收敛 + parity 签收 + 真实端到端任务过新栈，见 B15 与
> `OPEN_QUESTIONS.md` §1 的完整论证）。本报告覆盖本波次实际迁移面。

## 0. 交付摘要

| 项 | 结果 |
|---|---|
| 账本 | done **69 → 103**、wip 23、todo 437（`cargo xtask parity --check` OK） |
| 新增 Rust | 约 **11.5k 行**（含测试与 7 个 criterion 基准文件） |
| 测试 | workspace 46 套件全绿（config 41 / session 51 / serve 87+3 / assembly 10 / host 15+5 / 既有 572+ 不动） |
| 质量门 | clippy `-D warnings` 0、fmt 0、xtask deps/parity 0、`cargo build --release` 0 |
| 冒烟 | release 二进制真实进程 + curl 走门：**10 项契约行为**逐条通过（§3） |
| 审查 | Agent-Opt-Review：无 BLOCKER；3 MAJOR / 8 MINOR / 7 NOTE，**MAJOR 与建议 MINOR 全部修正并回归**（`docs/optimization-review.md`） |
| 契约漂移 | 修正后清单为空（漂移 9 项中 5 项为已记录偏差、1 项修正、3 项属裁剪面） |

## 1. 行为一致性对照表

### 1.1 HTTP 面（本波次注册的路由与门）

| 行为 | Go 侧 | Rust 侧 | 一致 | 差异与原因 |
|---|---|---|---|---|
| loopback 门 Host 检查 | 403 `loopback.host_rejected`（loopback.go） | gate.rs，golden 与 Go 实录**逐字节一致** | ✓ | — |
| loopback 门 Origin 检查（变更时） | 403 `loopback.origin_rejected`；空 Origin 仅 GET/HEAD 放行 | 同上，golden 对拍 | ✓ | — |
| loopback 门凭据 | cookie-only 常量时间比较；403 `loopback.unauthorized` | 同上（XOR 折叠实现，长度无关测试钉住） | ✓ | — |
| 门未配置（无 origin/token） | 500 `loopback.misconfigured` 恒拒 | 同上 | ✓ | — |
| Reason 信封 | `{code,error,params}`，params 键序确定（fail.go） | refuse.rs 字段序+键序测试 | ✓ | — |
| 共享拒绝码表 | refuse_common.go 19 码 | 19 码常量表 | ✓ | — |
| decodeBody | 64KiB MaxBytesReader + DisallowUnknownFields + badBody 不泄露解析细节 | 流级真实截断 + serde 拒未知字段；撒谎 CL 与 chunked 超限测试 | ✓ | — |
| auth none/token/invalid | deny 401 纯文本 `Unauthorized\n`；invalid deny-all | 同上 | ✓ | password 模式裁剪：构造为 deny-all（不伪造登录页），见 §6 |
| mutationGate | 非 GET/HEAD/OPTIONS 需 launch token；host-gated 放行；403 `auth.launch_token_required` | 同上（标记经门带下） | ✓ | — |
| csrfGuard | POST 必须 application/json；`/attachments/upload`、`/appearance/wallpaper` octet-stream 例外；415 | 同上 | ✓ | — |
| `/auth/token` bootstrap | ≤8KiB、单 JSON 值、常量时间、204+cookie | 同上 | ✓ | — |
| token 模式 query 凭据 | 302 清参 + cookie；safeRedirectTarget 消毒 | 同上 | ✓ | — |
| hostGuard | 421 `serve.host_rejected`（allowlist） | 同上 | ✓* | wildcard bind 的接口地址展开未实现，fail-closed（ROUTE-012 wip） |
| 监听 | ReadHeaderTimeout 10s / IdleTimeout 120s / drain 10s / ErrServerClosed→nil | listen.rs 同值；shutdown 信号即关空闲连接（hyper-util GracefulShutdown，审查 m-3 修正） | ✓ | idle 检查粒度 10s 轮询（近似） |
| FindPage | 显式 dir 必须 index.html → exe 旁 → `../desktop-tauri/dist`、`desktop-tauri/dist` → None | 同序（page.rs 测试 + 冒烟命中真实 dist） | ✓ | 契约文档 §3 的候选路径陈旧 → BREAK-002；条件请求（If-Modified-Since/Range）未实现（n-3/n-4 注记） |
| withPage 文件优先 + content-type 表 | page.go:64-69 固定表 | 同上 | ✓ | — |
| `/rt/{id}/` 未知 | 404 `request.not_found {kind:"runtime", name:id}` | 同上 | ✓ | — |
| routeDefault | 无 runtime → 503 `hub.no_runtime_open`；GET / + page + 已认证 → 页面 | 同上（冒烟：/sessions 与 /events 落 503，/ 落页面） | ✓ | 审查 M-1 修正：`/sessions` 从 hub 层移除 |
| watchTreeWrites 触发集 | hub_sync.go:71-90 路径集 + 2xx | 同集（router.rs `tree_write`） | ✓ | — |
| SSE 帧 | `: connected\n\n` 立即 flush；`id:` 仅编号帧；15s `: ping`+真 watermark 帧 | eventstream.rs（golden SSE 帧逐字节一致） | ✓ | — |
| Last-Event-ID 恢复 | header/query；attach vs resume | 同上 | ✓ | — |
| `/events/replay` | `{frames, complete, watermark}`，frames 永不 null | 同上 | ✓ | events mux 家族裁剪（§6） |
| POST /runtimes | decodeBody → Open | 同上；无 open 机器 → 500 http.Error 文本（Go 对不可构建 pane 的 catch-all 形状） | ✓* | 冒烟值 `runtime open is not part of this build`——诚实的未迁移占位，cutover 前由 Go 承担 |
| CORS | 无 CORS，same-origin | 无 CORS 面 | ✓ | — |

### 1.2 SSE 词汇与编号

| 行为 | 一致 | 差异 |
|---|---|---|
| 36 kind 名与顺序（eventwire manifest，golden 门） | ✓ | — |
| droppable 集（ToolProgress 除 reasonix.subagent.status、Text、Reasoning、CompactionProgress） | ✓ | — |
| transport-scoped seq；ResetSession 保计数丢尾巴 | ✓ | — |
| 回放 8MiB 字节预算逐出 + since() 完整性 | ✓ | — |
| 订阅者 4MiB 预算逐级摘除（先 droppable 后最老）+ 64 深 live window | ✓ | — |
| 五种订阅（From/InitialNotices/Attached/Resume），initial-notices 未编号副本每会话一次 | ✓ | — |
| gap 帧 seq=第一可信帧；Attached 前置未编号 watermark | ✓ | — |
| 信封字段集/omitempty/externalizable | ✓ | contract::eventwire 持有（SSE-004 done） |
| Go broadcaster 的定价/货币/workMs 边状态 | — | 裁剪面（B-005/SSE-011），非漂移（审查 n-5） |

### 1.3 host 面

| 行为 | 一致 | 差异 |
|---|---|---|
| argv 优先级（schedule exec → flags） | ✓ | mac/tree handoff 机器未迁（槽位保留，识别到即 exit 1 + 日志） |
| flag 集（10 旗标，Go flag 语义） | ✓ | — |
| 退出码 0/1/2 | ✓ | — |
| 12 相位启动日志（`reasonix-studio-host: startup phase=<n> elapsed_ms=<m>`） | ✓ | boot.* 转发随 runtime 切片 |
| 握手行（version 1 / origin / token，字段序，64 hex） | ✓ | 冒烟逐字节形状核对 |
| 凭据每启动铸新（32B hex） | ✓ | — |
| loopback bind tcp4 127.0.0.1:0 | ✓ | — |
| announce 顺序（bind→provider-setup→announce→serve） | ✓ | provider-setup 机器为槽位（相位行如实发出） |
| 租约 EOF → 干净 exit 0 + 端口释放 | ✓* | 分类窄化：terminal=unsupervised，file 与 pipe 均按 supervised（std 无 FFI 不可辨 handle 类型；OPEN_QUESTIONS §7；壳 spawn 恒 pipe，生产不可达） |
| act 行（relaunch/quit/show，互斥成行；relaunch 仅 Linux） | ✓ | 触发器（toast/notify）未迁 |
| `-instance-id` | ✓* | canonicalize 窄化（marker 存在时等价；home 未建时按拼写，OPEN_QUESTIONS §8）；Windows 哈希分隔符经审查 M-3 修正为与 Go `filepath.Clean` 逐字节同形 |
| hostServeConfig 清零（4 字段） | ✓ | 类型化 `host_serve_section()`；路由只吃铸新凭据，用户 serve auth 不达 hub |
| 优雅排空 | ✓ | — |
| 无 page → kernel-only + 日志行 | ✓ | — |

### 1.4 配置面（S1 子集）

| 行为 | 一致 | 差异 |
|---|---|---|
| 分层 user→project→defaults；缺文件静默 | ✓ | flag 层属 web 命令（未迁） |
| 未知键静默透传；已知键类型错误整层丢弃 + Go 原文警告 | ✓ | — |
| serve 段归一化（trim/lower、空=none、非法=invalid-deny-all） | ✓ | — |
| userGlobals（项目层不得设 [serve]/[storage]） | ✓ | §10 全矩阵未迁（CONFIG-008 wip） |
| desktop telemetry/metrics/tray 归一化 | ✓ | — |
| AutoWorkspaceDir 根链（REASONIX_STATE_HOME→[storage] state→home） | ✓ | cache/locks 根未迁 |
| 用户配置选择链（primary→legacy OS-support→XDG） | ✓ | isUserConfigPath/managed scoping 未迁（PATHS-005 wip） |
| CONFIG-035 分歧之一 | ✓ | auth_mode：配置层零值（gate 归一化），example 的「token 默认」是 CLI web 的按命令分层——按 Go 源定谳 |

## 2. L0 正确性修复清单

| ID | Go 侧问题 | Rust 侧修复 | 影响面 |
|---|---|---|---|
| OPT-001 | （port 时风险）慢订阅者阻塞广播 | 每订阅者 4MiB 有界队列 + 逐级摘除；`emit` 零 await；10k emit 对停滞读者收敛测试 | SSE 吞吐下的广播健康 |
| OPT-002 | 连接任务泄漏风险 | 每连接任务 `select!` 于帧/keepalive/断开；断连即收口；审查 m-5 后补真实 TCP 断连→订阅者归零测试 | 长连接服务的任务数稳定性 |
| OPT-003 | net/http 每 handler panic 恢复为连接级 500 | 自写 CatchPanicLayer（~60 行，未引 tower-http）：500 text/plain + tracing::error | 任何 handler panic 不打穿进程 |
| OPT-004 | MaxBytesReader 语义须真实截断 | 读侧限制 reader；撒谎 CL 与 chunked 超限均 400 `request.bad_body` | 请求体上限不可绕过 |
| OPT-005 | 404/405 码面 | MethodRouter fallback + routeRuntime 404 进统一信封 | 拒绝面一致性 |
| OPT-006 | 树构建容忍语义 | 逐条目失败跳过+汇总日志；目录不可读→空（对照 Go hub_tree 测试） | 侧栏树在半损坏目录下可用 |
| OPT-007 | 路径规范化第三份实现的风险 | 全部身份比较复用 `state::canonical_session_path` | Windows 大小写/链接折叠单一真相 |
| OPT-008 | 租约 EOF 与信号竞态 | 单一 CancellationToken；两路只 cancel；exit 0 幂等 | 双源关闭的退出码确定性 |
| OPT-009 | act 行撕裂 | std Mutex 短临界区（无 await）+ 单 write_all；审查 m-6 后补 64 线程并发成行测试 | 壳→内核控制线的完整性 |
| OPT-010 | 配置未知键行为漂移 | 未知键透传/坏层丢弃/警告文本逐字对齐 Go | 配置面行为对等 |
| 审查 M-1 | （port 期引入）`/sessions` 注册在 hub 层 | 移除；无 runtime 落 routeDefault 503——恢复 Go 拓扑 | 无 runtime 时的可探测面 |
| 审查 M-2 | （port 期引入）项目配置双文件合并 | 存在性探测二选一（根文件胜，仅 IsNotExist 许回退；symlink_metadata 对齐 Lstat） | 双文件工作区的生效配置 |
| 审查 M-3 | （port 期引入）instance-id Windows 哈希分隔符被改写 | 哈希入参用 OS 分隔符 Clean（session::paths::clean）+ Go 字面量对照测试 | 壳 profile/单实例锁 scoping 的身份一致性 |
| 审查 m-2 | serve token 生成 panic（Go 是 error→exit 1） | `generate_token` 返回 Result；`build_router` 失败降级 invalid-门（authInvalid 语义） | CSPRNG 受限环境不崩进程 |

## 3. L1/L2 优化清单（含证据）

| ID | 级别 | 问题 | 方案 | 证据 | 收益评估 |
|---|---|---|---|---|---|
| OPT-011 | L1 | 门检查热路径分配 | 策略一次校验+借用输入；手写常量时间比较 | criterion：零分配版 40.2ns vs 直译重建版 265.7ns（审查复测 44.8/566ns）；常量时间比较 25.5ns 且长度无关 | 每请求省 ~0.2µs（门在每请求最外层） |
| OPT-012 | L1 | 事件信封序列化 | 复用 eventwire 类型 + droppable 判定先行不进回放日志 | criterion：droppable 599.7ns vs numbered 827.1ns（无订阅者）；停滞订阅者下 numbered 49.3µs（逐出路径）/droppable 619.5ns | 高频 delta 帧零回放成本；**Go 基线未量化**（标注） |
| OPT-013 | L1 | SSE 逐帧 flush 的写次数 | 单帧单缓冲一次写（不跨帧攒） | criterion：122.1ns vs 直译两次分配 463.0ns（审查复测 124.9/525.4ns）；帧边界行为测试钉住 | 每帧 3.8× 渲染加速；逐帧 flush 语义保持 |
| OPT-014 | L1 | 树构建重复排序/每行重读 sidecar | 一次收集+单次排序+每目录一次 title 快照 | criterion：500 会话 66-94ms vs 直译 103-310ms（审查复测 46.1ms）；**Go 基线 89.1ms（100 根，本机实跑 `go test -bench`，口径含 legacy-listings/v4 探测，仅参考）**；100 根 28-34ms | 树构建约 2-3× |
| OPT-015 | L1 | 相位日志开销 | 常量名相位表 + 直接 stderr 写（不走 tracing） | 复杂度分析 O(1)/相位、12 次系统调用/启动（profile-host.md 留痕，m-7）；未量化（启动期一次性行，不设基准） | 可忽略（诚实标注） |
| OPT-016 | L1 | 配置分层加载重复解析 | 计划预设 Value 树 merge；实测 **direct 逐层解码 18.4-20.4µs vs merge 31.5-35.3µs（负收益 54-92%，两轮一致）→ 按证据保留 direct** | benches/{load,direct,merge}.rs 三文件落盘；审查 m-8 后本机可直接复现 | 计划方案被证据否决——机制本身达成 OPT-016 的目标（避免每层重复反序列化） |
| OPT-017 | L2 | refuse 码散落 | serve::refuse 单一模块（19 码表+枚举+writeErr 映射序） | 全部路由走统一信封；clippy 0 | 维护面收敛 |
| OPT-018 | L2 | DTO 重复 | session::model 单一 DTO 集，serde 就地派生 | 无第二份声明可查 | 维护面收敛 |
| OPT-019 | L2 | os.Exit 散落 | ExitCode 归一产出，main 只 return | cli 测试钉 0/1/2 三主路径 | 可测试性 |
| OPT-020 | L2 | host 清零逻辑位置 | config 类型化 `host_serve_section()` | 清零后 4 字段恒默认测试 | HOST-018 类型层不可违 |

预算核对：serve 3 条 L1、session 1 条、host 1 条、config 1 条 —— 全部 ≤8。无伪优化
（unsafe/泛型化/去错误处理/clone 掩盖：审查专项核查通过）。

## 4. 基准结果汇总

| 基准 | 直译版 | 优化版 | 备注 |
|---|---|---|---|
| gate_check（serve/gate.rs） | 265.7ns（复测 566ns） | **40.2ns**（复测 44.8ns） | 门三连+常量时间比较 |
| constant_time_eq | — | 25.5ns（长度无关钉住） | 安全面零回退 |
| frame_render（serve/eventstream.rs） | 463.0ns（复测 525.4ns） | **122.1ns**（复测 124.9ns） | 单缓冲一次写 |
| sse_write_1000_frames | — | 127.7µs | 管线端到端 |
| emit（serve/broadcaster.rs） | numbered 827.1ns | droppable 599.7ns（无订阅者） | 停滞订阅者：numbered 49.3µs（逐出） |
| tree_500_sessions（session/tree.rs） | 103-310ms | **66-94ms**（复测 46.1ms） | Go 基线 89.1ms（100 根场景，口径差异注记） |
| tree_100_roots | — | 28-34ms | 同上 |
| config load（direct，三层小文件） | — | 18.4-20.4µs 纯解码 / 163.4µs e2e | merge 方案 31.5-35.3µs（负收益，未采纳） |

未量化条目：OPT-012 的 Go 对照（Go 无对应 bench）、OPT-015（启动期一次性行）。
冒烟压测（oha/wrk）：本机未装，以 curl 功能冒烟代（§5）；吞吐压测待 CI/装机环境。

## 5. 冒烟结果（release 二进制，真实进程）

启动：握手行 `{"version":1,"origin":"http://127.0.0.1:<port>","token":"<64hex>"}`
+ 12 相位日志按冻结顺序。curl 走门（带/不带 cookie、错 Host、无 Origin）：

1. 无 cookie GET / → **403 `loopback.unauthorized`**（信封逐字节）
2. 错 Host → **403 `loopback.host_rejected`**
3. 无 Origin 变更 → **403 `loopback.origin_rejected`**
4. 带 cookie GET / → **200**（FindPage 命中 `desktop-tauri/dist` 真实 Vue 页）
5. GET /tree → **200 `[]`**（空状态家）
6. GET /sessions → **503 `hub.no_runtime_open`**（M-1 修正后，与 Go hub 拓扑一致）
7. GET /events → **503 `hub.no_runtime_open`**
8. POST /runtimes（cookie+origin）→ **500** 纯文本诚实占位
9. 未知路由 → **503 `hub.no_runtime_open`**
10. 租约 EOF → **exit 0**（集成测试断言；冒烟期后台进程同样随 stdin EOF 干净退出）

## 6. L3 backlog 摘要与拒绝理由

全部 L3 见 `docs/optimization-backlog.md`。拒绝理由一栏：本波次不改 API 形状/错误码/
schema/配置键——SSE 词汇的 snake/camel 混用、`truncated` 双字段、`err` vs `error`
键差、404 信封形状、门 cookie-only、配置未知键行为等全部维持 Go 现状（wire 稳定面）。
本波次裁剪面（后续波次范围，非 L3）：password 鉴权、device 门+hostOnly、events mux、
wire log/trajectory、sink 包装层（Coalesce/CostQuote/NoticeLedger/MarkEmbeddedDiffs）、
/tree live 流、provider/tools/agent/safety/ext/platform/tui/acp/cli 全量、host 平台机器、
config 全键面、cutover（B-017）。

## 7. 审查摘要（docs/optimization-review.md）

- 无 BLOCKER；3 MAJOR / 8 MINOR / 7 NOTE；结论「需修正后放行」。
- MAJOR 3 条（`/sessions` 注册面、项目配置双文件合并、instance-id Windows 分隔符）
  全部修正并回归；建议 MINOR（launch 优先、token panic、config Temp 依赖）同批修正；
  m-4/m-5/m-6 以补测试收口；m-7 以 profile-host.md 留痕收口。
- 伪优化/并发纪律专项核查：**无发现**——全树 forbid(unsafe)、std Mutex 临界区无 await、
  队列全部有界、emit 不阻塞、常量时间比较有长度无关测试、无 L3 变更。
- NOTE 7 条为记录性（死代码下波清、路径语言两份待 PATHS 收编、页面条件请求待补、
  broadcaster 定价边状态属裁剪、transport 帧建模形状差异、冻结 API 的后续锚位）。

## 8. Go ↔ Rust 文件映射表（本波次）

| Go 源 | Rust 侧 | 状态 |
|---|---|---|
| internal/contract/config/{load*,serve*,desktop*,storage_roots,expand*,auto workspace} | crates/config/src/{load,serve,desktop,paths}.rs | 子集 done（CONFIG-001/002/003/004/008/022/035 wip） |
| internal/base/fileutil + paths 的用户配置链 | crates/config/src/paths.rs（user_config_path/reasonix_home_dir） | PATHS-005 主体 |
| internal/state/workspacelist/list.go | crates/session/src/workspacelist.rs | done（读侧） |
| internal/frontend/serve/hub_tree.go（读侧）+ hub.go（findSession/publish 键） | crates/session/src/{tree,listing,hub}.rs | done（读侧） |
| internal/frontend/serve/sessions_list.go + session_content.go（modified） | crates/session/src/listing.rs | done（读侧） |
| internal/frontend/serve/{loopback,host_guard,launch_token,auth}.go | crates/serve/src/{gate,auth}.rs | done（裁剪：password/device） |
| internal/frontend/serve/fail.go + refuse_common.go | crates/serve/src/refuse.rs | done |
| internal/frontend/serve/listen.go | crates/serve/src/listen.rs | done |
| internal/frontend/serve/page.go | crates/serve/src/page.rs | done（条件请求未迁） |
| internal/frontend/serve/broadcaster.go + eventstream.go | crates/serve/src/{broadcaster,eventstream}.rs | done（裁剪：mux/定价边状态） |
| internal/frontend/serve/serve.go + hub.go（路由面/middleware） | crates/serve/src/router.rs | 子集（hub 级 5 路由 + per-runtime 集） |
| internal/frontend/serve/sessions_list.go（Server 位） | crates/serve/src/router.rs::session_routes | done（per-runtime 位） |
| cmd/reasonix-studio-host/main.go + startup.go + owner.go + launchroot.go | apps/reasonix-studio-host/src/* + crates/assembly/src/host.rs | 启动面 done（平台机器未迁） |
| internal/platform/instanceid | crates/assembly/src/host.rs（instance_id_*） | done（canonicalize 窄化，OPEN_QUESTIONS §8） |
| internal/contract/config.Load（host 视角） | crates/serve/src/boot.rs::load_config | done（审查 M-2 修正后语义对齐） |
| （未迁移）internal/{model,runtime,tools,safety,ext,platform,frontend/{cli,tui,acp}}、assembly/boot 全量 | crates/{provider,agent,tools,safety,ext,platform,tui,acp,assembly}（骨架） | 后续波次（backlog B-007..B-015） |

## 9. 偏离 idiomatic Rust 或偏离原行为的地方及理由

| 偏离 | 理由 |
|---|---|
| `generate_token` 失败降级 invalid-门而非返回错误 | `build_router` 冻结签名不可变；CSPRNG 失败时「拒绝一切」= Go authInvalid 语义，等价安全 |
| 租约分类窄化（terminal-only unsupervised） | forbid(unsafe) 下无法读 Windows handle 类型；壳 spawn 恒 pipe，文件 stdin 不可达（OPEN_QUESTIONS §7） |
| instance-id canonicalize 窄化 | 冻结依赖策略禁 host/assembly→state 直连；marker 存在时等价（OPEN_QUESTIONS §8） |
| schedule-exec/strip-package-grants 识别后 exit 1 | 机器未迁，不伪造成功（OPEN_QUESTIONS §4） |
| hyper-util GracefulShutdown + 轮询 idle 看门狗替代 Go net/http 内建 | Rust HTTP 栈差异；语义对齐（10s header / 120s idle / 10s drain，m-3 修正后空闲连接随信号关闭） |
| transport 帧（watermark/gap）以 Event{kind} 建模而非独立类型 | 线上字节与 Go 完全一致（审查 n-6）；两建模形状等价 |
| 手写 hex / 常量时间比较 / CatchPanicLayer 而非引依赖 | 供应链政策：最小依赖面（tech-choice.md） |
| FindPage 以 Go 源为准（desktop-tauri/dist）而非契约文档 | oracle 优先级：源>文档；BREAK-002 记录 |

## 10. 被改写的部署/配置资产清单

| 资产 | 处置 |
|---|---|
| Dockerfile / docker-compose.yml | **不存在**（桌面产品，无容器面）——无可改写项 |
| Makefile | 增补 `kernel-check` 目标（Rust 内核工作区五门：fmt/clippy/test/parity/deps；`KERNEL_TARGET_DIR` 沿用 Tauri 的 Desktop-off 约定）；其余目标不动 |
| CI（.github/workflows/ci.yml） | 不动：本波无 cutover，Go 门仍是发布权威；Rust 内核 CI 面随 INFRA-002（六目标发布管线，待用户 secrets） |
| README / README.zh-CN | 不动：内核重写用户不可见（Go 仍是 live 后端）；cutover 波次改写 |
| `.cargo/config.toml` / deny.toml / rust-toolchain.toml | 不动（deny.toml 的 licenses allowlist 已覆盖新增依赖 tokio/axum/hyper/tower/toml/tokio-util——MIT/Apache/ISC） |
| 依赖（根 Cargo.toml） | 新增 workspace 条目：tokio、tokio-util、axum、bytes、tokio-stream、toml、criterion（dev）；移除解析失败的 futures-core 条目 |

## 11. OPEN_QUESTIONS 摘要（OPEN_QUESTIONS.md）

1. **Go 归档/删除未执行**：与任务规则 2 的偏差及完整论证（live 后端 + Go 反射权威门 +
   评测夹具不可归档）；替代安全网 = `rust-migration` 分支。
2. `.migration-backup/` 未跟踪目录保持原样。
3. 本波次裁剪面清单（账本未收敛部分）。
4. schedulerun / strip-package-grants 的保守失败语义。
5. axum 0.8 选型（tech-choice.md）。
6. 握手凭据随机源 = 已冻结的 getrandom + 手写 hex。
7. 租约分类窄化（IsTerminal 近似）。
8. instance-id canonicalize 窄化。
9. **并行会话事件**：工作树出现另一会话的未提交改动（desktop-tauri remote/share +
   Go share 面）；一次 `git add -A` 卷入后已 `git reset --soft` 拆分、内容零改动原样保留；
   此后本迁移只用显式路径 add。

## 12. 测试与冒烟结果、已覆盖与未覆盖行为

**覆盖（本波次新增 ~230 项断言）**：config 41（键面/分层/未知键/警告文本/auth 归一化/
根链/选择链）；session 51（workspacelist 容错/树/行形状/canonical 键/title）；serve
87+3（门四码 golden 逐字节、auth 三模式、信封与 19 码、decodeBody、监听与排空、
broadcaster 全机件（Go 20 用例移植）、路由 405/404/503、真实断连回收）；assembly 10
（instance-id/launch root/Go Clean 字面量）；host 15+5（flag 语义/凭据/握手字段序/
act 成行与并发/相位格式/租约分类/CLI 退出码/租约 EOF/（unix）SIGTERM）。

**未覆盖/未迁移**：写侧全部（submit/cancel/approve/new/fork/rewind/compact…）、
provider 流式、工具执行、审批/沙箱、events mux、wire log/trajectory、password/device 鉴权、
/tree live 流、modhost/embedder、tray/notify/schedule/update/crash log、
contract/config 全键面、cli/tui/acp 面。这些由 Go 侧继续承担（B15 未切换）。

## 13. 上线前人工确认清单

- [ ] **cutover 前置（B15）**：账本 done 覆盖写侧与鉴权全部行；golden fixture 以
      「无 runtime」宿主与「有 runtime」宿主各录一轮（当前 7 例为 runtime-open 真值，
      Rust 重放按形状断言——审查遗留事项）。
- [ ] DB/投影：三个 SQLite 投影库仍由 Go 写成——切换前确认「可丢弃重建」契约在新栈
      上实际执行过一次重建（本波次未触 DB）。
- [ ] 时区/时间：`Timestamp` 零值 `0001-01-01T00:00:00Z` 与 chrono FixedOffset 的
      会话目录时间戳在跨时区机器上对拍（本波次未触写侧时间戳）。
- [ ] 整数溢出：seq（i64）与字节数预算（usize）在超长会话（>10^9 帧）下无溢出面——
      已由类型选择覆盖，切换前跑一次超长会话回放。
- [ ] JSON 数字精度：usage/cost 字段经 serde i64/f64——与 Go encoding/json 的
      大数/精度行为对拍（PMSG golden 已覆盖消息面；SSE usage 帧待写侧波次对拍）。
- [ ] 事务隔离：无业务 SQL 面（投影库可重建），无隔离级别漂移。
- [ ] 时钟精度：keepalive 15s 与 idle 120s 用 tokio 轮询实现（10s 粒度）——确认
      反代环境（30-60s 空闲超时）下 keepalive 足够密。
- [ ] goroutine 语义差异：Go 的订阅者 pump goroutine ↔ Rust per-connection task——
      断连回收已有测试；长稳 soak（>24h 挂机 SSE）建议 cutover 前实跑。
- [ ] 优化引入的行为微差：decodeBody 对 >64KiB 且带合法 Content-Length 的体的拒绝
      （Go 无 cap 的裸 Decode 只在 openRuntime 等处不同——见审查冲突 3 的采纳记录）；
      withPage 缺 If-Modified-Since/Range（前端无强依赖，切换前实测一次页面加载）。
- [ ] **并行会话协调**：工作树中另一会话的 remote/share 改动（OPEN_QUESTIONS §9）
      与本迁移无文件交集；其提交由其会话自行完成，勿混入迁移分支。
- [ ] Windows 服务化/杀软：本机 `%TEMP%` 拒写与 `os error 5`（tauri-dev-os-error-5
      备忘录）表明环境策略会影响新二进制——CI（干净 runner）是权威验证位；本机复跑
      用 `TMP/TEMP=target\tmp-tests`。

## 14. 结论

本波次把 Rust 内核从「M2 骨架 + state/contract」推进到「**host 可启动、门与 SSE 全机件、
hub 读侧可用**」的可验证增量：账本 done +34 行、全部质量门绿、真实进程冒烟 10 行为通过、
独立审查闭环。Go 源按其自身收敛条件继续保留为 live 后端；删除条件（账本收敛 + parity
签收 + 真实端到端过新栈）未达成，如实不执行。后续波次入口：`docs/optimization-backlog.md`
B-005..B-017，账本 todo 437 行。

---

# 追加：波次 18（工具 ABI + 权限判定核）

## 0. 摘要

| 项 | 结果 |
|---|---|
| 账本 | done **110 → 117**、wip 27、todo 428 |
| 新增 Rust | ~6.9k 行（tools 11 源 + 5 测试；safety/permission 12 文件） |
| 测试 | tools 63 绿 + safety 57 绿 + 8 锚位 #[ignore]（Go oracle 表逐用例） |
| 审查 | 追加审查放行：零 MAJOR/MINOR；伪优化/并发/错误身份三专项无发现 |

## 1. 行为一致性对照（关键面）

| 行为 | Go | Rust | 一致 |
|---|---|---|---|
| permrule 三形态 Parse（含 legacy "=literal"、malformed→ok:false） | permrule.go | permission/permrule.rs（17 用例） | ✓ |
| Decision 三态 + ParseDecision unknown→Ask | permission.go | decision.rs | ✓ |
| Policy 求值序 deny>ask>session-allow>ask>allow>reader-allow>subjectRequiresHuman>mode | policy.go | policy.rs（policy_decide_table） | ✓ |
| write fence 四敏感工具 + bare-grant 收窄 | write_fence.go | write_fence.rs | ✓ |
| Gate：headless Ask→Allow、interactive remember/decline、denial 命名 | gate.go/verdict.go | gate.rs | ✓ |
| session grant 校验（host-recorded only） | session_grant_valid.go | session_grant_valid.rs | ✓ |
| inline-code argv 表（9 解释器拼写）+ indirect-execution blocker | shellsafe/inline_code.go + bash_approval.go 纯 argv 半 | inline_code.rs + bash.rs | ✓ |
| Tool trait + 能力面（ReadOnly 批量语义、Preview 同解析路径、contextual fail-closed） | tool.go | tools/abi.rs（CapabilitySet） | ✓ |
| Registry（restriction/replace-prefix/withdrawn/disabled/held MCP 优先序） | tool.go + 4 文件 | tools/registry.rs + tests/registry_policy.rs（20 用例） | ✓ |
| argument envelope（uniqueJSON 三态 + Draft-7 子集） | arguments.go | tools/arguments.rs（20 用例） | ✓ |
| Refusal/Remedy 渲染逐字 + BlockedError 身份 | refusal.go/blocked.go | tools/refusal.rs | ✓ |
| NestedCallID canonical 1-based | invoke.go | tools/invoke.rs | ✓ |
| schema canonicalize（键序/required 数组化/dependentRequired 清洗） | provider.CanonicalizeSchema | tools/schema.rs（8 用例） | ✓ |
| provenance 读类型不读名、永不零值 | provenance.go | tools/provenance.rs | ✓ |

**诚实锚位**（不伪造）：bash AST 级判定（wrapper 剥离/复合叶/redirect 归一）→
`BashAnalysis` trait + fail-closed StaticBashAnalysis + 8 锚位测试；annotations/runtime
MCP bind、target access、peer/settings/elicit 等 trait 锚位随签名留注释。

## 2. 修复与优化

- L0：无新发现（迁移波次）。
- L2：OPT-T001 错误分类单点化、OPT-T002 schema 快照一次化（once-per-tool 测试钉住）、
  OPT-T003 duplicate-key 前置；permission 侧 deny 规则免二次解析、正则→手写校验免 regex。
- 主 Agent 决策三项：BashAnalysis 引擎 seam 为冻结面；origin 显式参数化；
  CapabilitySet TypeId-downcast（供应链内无 anymap）。

## 3. 文件映射（波次 18）

| Go | Rust |
|---|---|
| internal/contract/tool/{tool,registry*,replace_prefix,withdrawn,disabled_*,held_mcp}.go | crates/tools/src/{abi,registry,withdrawn}.rs + tests/registry_policy.rs |
| internal/contract/tool/{arguments,shell_execution,refusal,blocked,invoke,provenance,roots,reach,contract}.go | crates/tools/src/{arguments,shell,refusal,invoke,provenance,roots}.rs + schema.rs |
| internal/contract/permrule/permrule.go | crates/safety/src/permission/permrule.rs |
| internal/safety/permission/{permission,verdict,gate,write_fence,rule_*,subject,session_grant_valid,origin_ctx,policy_options}.go | crates/safety/src/permission/{policy,gate,write_fence,decision,session_grant_valid}.rs |
| internal/safety/permission/{inline_code(→shellsafe),bash_approval 纯 argv 半}.go | crates/safety/src/permission/{inline_code,bash}.rs |
| internal/safety/permission/bash_{approval,decompose,readonly,redirect}.go AST 半 | 锚位：BashAnalysis seam + bash_anchors（8 #[ignore]） |

## 4. 累计状态（波次 17+18）

- 账本：done **117** / wip 27 / todo 428（起点 done 69）。
- 分支 `rust-migration` 累计 18 提交；51 套件全绿；clippy/fmt/deps/parity 全 0。
- 解锁下一片：bash AST 引擎（装入 `set_bash_analysis` 翻 SAFETY-008/009）→
  builtin 工具目录（TOOL-024..035）→ agent 循环（B-008）→ serve 写路由。

---

# 追加：波次 19 切片 1（bash AST 引擎）

## 摘要

| 项 | 结果 |
|---|---|
| 账本 | done **117 → 119**（SAFETY-008/009）、wip 25、todo 428 |
| 新增 Rust | ~2.6k 行（词法核结构改造 + shellparse_analysis + safety::shellsafe + 引擎装配 + network AST 化） |
| 测试 | core 69 + safety 66 全绿；**10 个锚位测试去 ignore 全绿**（Go oracle 表逐用例）；workspace 51 套件全绿 |

## 关键语义对齐（oracle 发现）

1. 词法核 `&&`/`||`/`|` 推左操作数——复合语句的 BinaryCmd 形状（此前折成单条 Control 语句）。
2. Bash prefix 规则经 `bashPrefixMatches` 单独作答：复合语句永不落回 raw glob（Go permission.go 的 return 语义，波次 18 移植时的 if-let 链漏掉了）。
3. redirect 归一按原始拼写判定（`$null`/`NUL`/`2>&1`），span 由词法核携带。
4. `$(...)` 捕获内文：resolved-substitution（双引号内、单条静态只读子命令）在只读分类中可解析；未加引号的 `$(` 保持 NestedExecution fail-closed。
5. 网络操作数走 word skeleton（动态段置 NUL、brace 变体展开、PowerShell provider 前缀剥离），Windows 判定与 Go `holdWindowsShellPaths` 测试语义一致。

## 引擎 seam 收口

`BashAnalysis` trait（波次 18 冻结）+ `AstBashAnalysis`（默认引擎）+ `StaticBashAnalysis`
（保留为显式覆盖通道）。8+2 锚位测试不再是锚位——它们是常规回归测试。

## 下一片

read-only builtin 工具（TOOL-028/050：confine/read_file/glob/ls/grep）进行中
（Agent-ReadonlyTools）；其后 builtin 写类（TOOL-029）→ jobs（TOOL-046..048）→
agent 循环（B-008）→ serve 写路由。

## 追加：波次 19 切片 2（只读 builtin 工具）

| 项 | 结果 |
|---|---|
| 账本 | TOOL-028/050/027/056 → wip（done 保持 119；todo 424） |
| 新增 Rust | ~5.1k 行（confine/secretpaths/network/pathgo/globengine/gitignore/walk/readfile/glob/listdir/grep + 67 测试） |
| 测试 | tools 150/150 绿；clippy 0；fmt 0 |

关键对齐：Go filepath Clean/Rel 行为级移植（lazybuf 体）；doublestar `**`/brace/转义；
read_file 编号窗口与 BOM/UTF-16/GB18030 级联（复用 core::fileencoding）；grep 两相
（tracked → ignored-pass）与 CJK peek 解码；confine 拒绝原文逐字（含 verbatim 路径纪律）。
裁剪面：code_index、rg 子进程引擎、confine 写侧（SessionDataGuard/managed/sessiontemp）、
PathResolver/FileViews——各自锚位。
