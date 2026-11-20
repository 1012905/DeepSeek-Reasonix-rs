# 优化计划（optimization-plan.md）

> 波次 17（host 启动切片）的优化条目。范围 = 本波次实际迁移的四个切片
> （config / session / serve / host）。级别定义：L0 正确性（必须）、L1 实现级（需证据）、
> L2 结构级（做）、L3 契约级（禁止，只进 `docs/optimization-backlog.md`）。
> 预算：每个 crate ≤ 8 条 L1，按「影响面 × 证据可得性」排序。

| ID | crate | 级别 | 问题 | Go 侧现状 | Rust 侧方案 | 证据方式 | 风险 | 依赖项 |
|---|---|---|---|---|---|---|---|---|
| OPT-001 | serve | L0 | 慢订阅者阻塞广播 | broadcaster Emit 永不阻塞，慢订阅者队列超 4MiB 预算即被摘除（broadcaster.go） | 每订阅者 `tokio::sync::mpsc` 有界通道 + 预算字节计数；超预算判定摘除，不等待 | 单测：慢消费者填满预算后被摘除，emit 不阻塞（有界性断言） | 摘除时机偏差破坏 SSE-005 | SSE-005 |
| OPT-002 | serve | L0 | 连接任务泄漏 | eventstream 每连接 goroutine + ticker，靠 done channel 收口；收口缺失即泄漏 | 每连接一个 tokio task，`select!` 于 keepalive/队列/取消 token；连接 drop 即取消 | 单测：client 断开后任务数回落（`tokio::runtime` 内任务计数或句柄 drop 证据） | 无 | OPT-001 |
| OPT-003 | serve | L0 | handler panic 静默吞/连接级 500 语义漂移 | net/http 每 handler panic 恢复为连接 500 + stderr 堆栈 | axum 顶层 catch 层（crate 内自写 ~10 行，不引 tower-http），panic → 500 text/plain + tracing::error | 单测：注入 panic 路由得 500；日志含 panic 摘要 | 行为与 Go 一致性靠测试钉住 | 无 |
| OPT-004 | serve | L0 | 请求体大小限制失效 | `decodeBody` 用 `http.MaxBytesReader` 64KiB，真实截断读取流 | 读侧包一层限制 reader（读满即断），不只看 Content-Length；超限 → 400 `request.bad_body` | 单测：Content-Length 撒谎/无 CL 的 chunked 超限体被拒 | 拒绝码须为 request.bad_body | ROUTE-024 |
| OPT-005 | serve | L0 | 非法方法/未知路由的错误码 | Go mux 405/404 + `request.not_found`/`request.method_not_allowed`（http.md §1/§4） | axum fallback + method_router 组合显式回答同码；拒绝码进统一信封 | 契约测试逐路由核对 404/405 码 | 无 | ROUTE-022/023 |
| OPT-006 | session | L0 | 树构建吞错导致半棵树 | hub_tree/workspacelist 对单条目读错容忍跳过、目录不可读返回空而非错误 | 移植容忍语义：逐条目 Result 吸收 + 汇总日志；目录级失败 → 空树 + `[]` 语义 | 单测：混合可读/不可读条目的树与 Go 测试用例对照 | 容忍面照抄 Go（不扩大不缩小） | SESS 列表行 |
| OPT-007 | session | L0 | 路径规范化漂移（Windows 大小写/符号链接） | findSession/树键用 canonical 路径（EvalSymlinks/Clean），大小写不敏感盘符折叠 | 复用 `crates/state` 已完成的 canonical path 语义（SESS 行 done），不重写第三份 | 现有 state golden 测试 + 树键一致性单测 | 重写会产生第三种路径语义 | state crate |
| OPT-008 | host | L0 | 租约竞态：EOF 与信号同时到达 | select on lease EOF / ctx；EOF → 干净 exit 0 | 单一关闭来源：`CancellationToken`，租约任务与信号任务都只 cancel；主循环 select 收尾，exit 0 且端口释放 | 集成测试：EOF 与 SIGTERM 两路都收敛 exit 0；重复 cancel 幂等 | 双源竞态 → exit 码漂移 | HOST-003/006 |
| OPT-009 | host | L0 | act 行撕裂 | act 写出由 mutex 串行（TestEachActTravelsAsItsOwnLine） | `std::sync::Mutex<StdoutLock>` 短临界区（无 await），逐 act 单 `write_all(line)` | 单测：并发 64 act 全部完整成行 | 用 tokio::Mutex 会把临界区拉过 await（伪优化禁区） | HOST-014 |
| OPT-010 | config | L0 | 配置键未知行为漂移 | config.md 冻结未知键处理与默认值矩阵 | 严格按 config.md 矩阵；unknown key 行为与 Go 一致（含 warn/refuse 分类） | 单测对照 config.md 用例表 | 无 | CONFIG 行 |
| OPT-011 | serve | L1 | 门检查热路径 | host/origin/cookie 常量时间比较三连（loopback.go/host_guard.go） | 保持三连序；常量时间比较用 XOR 折叠手写（不引 subtle 依赖）；门检查零分配 | criterion：`gate_check` ns 级基准（优化前=直译版，优化后=零分配版） | 手写常量时间比较需测试钉住长度无关性 | ROUTE-014 |
| OPT-012 | serve | L1 | 事件信封序列化开销 | eventwire.ToWire 每帧分配 + encoding/json 反射 | 复用 `contract::eventwire` 类型 + serde 序列化；droppable 帧不进回放日志（零拷贝判定先行） | criterion：单帧 ToWire+bytes 基准；与 Go `go test -bench` 无对照时标「未量化基线」 | 信封字段漏项破坏 SSE-004 | SSE-004 |
| OPT-013 | serve | L1 | SSE 写出逐帧 flush | Go 每帧 Flush（eventstream.go） | 同为逐帧 flush（契约），但合并 syscall：帧缓冲攒到一次 write（`\n\n` 边界）不得跨帧攒 | criterion：1000 帧写出耗时；行为测试：flush 边界（客户端按帧收到） | 攒写破坏逐帧语义 → 回退 | ROUTE-027 |
| OPT-014 | session | L1 | 树构建重复排序/分配 | hub_projects_benchmark（Go Benchmark 存在） | 一次遍历收集 + 单次排序；避免每条目 Vec 扩容 | criterion 移植 Go Benchmark 同场景，记录优化前后两轮数字 + Go 基线数字 | 无 | OPT-006 |
| OPT-015 | host | L1 | 启动相位时间戳开销 | 每相位一行 stderr（12 行/启动） | 短路：`tracing` 关闭时零格式化；相位表用常量数组驱动 | 复杂度分析（12 相位 O(1)）+ 未量化标注（启动期一次性行，不值得基准） | 无 | HOST-004 |
| OPT-016 | config | L1 | 配置加载重复解析 | Go 侧多层 load 各自 parse | 一次 parse 成 `toml::Value` 树，分层 merge 后定型强类型（避免每层反序列化） | criterion：三层加载基准；正确性由 CONFIG 单测钉住 | merge 语义偏差破坏 CONFIG 行 | OPT-010 |
| OPT-017 | serve | L2 | 错误分类散落 | refuse 码散在 fail.go/refuse_common.go/各 handler | `serve::refuse` 单一模块：dotted 码常量表 + `RefuseError` 枚举 + writeErr 映射序一函数 | clippy 无 warn；路由契约测试全部走统一信封 | 无 | OPT-005 |
| OPT-018 | session | L2 | DTO 复用 | 树/会话行各 handler 自拼 map | `session::model` 单一 DTO 集（WorkspaceTreeSnapshot/RuntimeSummary/SessionRow），serde 就地派生 | 编译期单一来源（无第二份声明可查） | 无 | OPT-006 |
| OPT-019 | host | L2 | os.Exit 散落 | Go 多处 os.Exit(code) 直呼 | `ExitCode` 归一个函数产出（host::finish(err) -> ExitCode），main 只 return | 单测：每失败路径 → 期望码 | 无 | HOST-003 |
| OPT-020 | config | L2 | host 清零逻辑位置 | hostServeConfig 在 host 里改 4 字段 | config 提供 `host_serve_section()` 类型化清零（类型层面不可能携带凭据） | 单测：清零后 4 字段恒默认 | 无 | HOST-018 |

## L1 预算核对

serve：OPT-011/012/013（3 条）✓ ≤8；session：OPT-014（1 条）✓；host：OPT-015（1 条）✓；
config：OPT-016（1 条）✓。

## 执行纪律

- L1 无证据不落盘；基准在优化**前**先跑直译版留数。
- 契约测试失败 → 回退该优化并记录（optimization-plan 表加「回退」注记）。
- 计划外新机会：子 Agent 只记汇报，不改代码。
