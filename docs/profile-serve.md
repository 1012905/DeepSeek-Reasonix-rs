# profile-serve（crates/serve 任务书）

## 范围

Go 侧：`internal/frontend/serve` 的门/信封/SSE/监听/页面发现 + 读侧路由子集
（关键文件合计约 3.4k 非测试行）。Rust 侧：`crates/serve`（当前 8 行 skeleton）。

## 必读（只读）

- `docs/contracts/http.md`（**全文**；重点 §0 拓扑、§2.6 门、§2.9 csrf、§3 绑定、
  §4 信封与拒绝码、§5 SSE 帧、§7 形状、§8 裁剪注记）
- `docs/contracts/sse.md`（**全文**；§1..§4 本波次，§5 sink 包装层裁剪）
- `docs/profile-serve.md`（本文件）、`docs/contract.md`（切片 S3 + 冻结边界 2/3/4）
- `docs/optimization-plan.md`（OPT-001..005/011/012/013/017）
- `crates/contract/src/eventwire.rs`（wire 信封类型——**复用，禁止重新声明**）
- `crates/session/src/lib.rs`（Hub 读侧——由 Agent-Session 同期落盘，签名以
  docs/contract.md §4 第 1 条为准）
- Go 源：`internal/frontend/serve/{loopback,host_guard,launch_token,fail,
  listen,page,broadcaster,eventstream,sessions_list}.go` + `serve.go`（路由注册与
  middleware 序）+ 两个测试文件（broadcaster/eventstream，直接 oracle）

## 实现范围（按 contract.md S3）

1. **loopback 门**（ROUTE-014）：`ListenLoopback` 等价（TcpListener bind `127.0.0.1:0`
   tcp4）+ `LoopbackGate::check_http`（Host → Origin(变更时) → cookie 常量时间比较，
   外到内短路；拒绝码四件套；misconfigured 恒拒）。GET/HEAD 空 Origin 放行。
2. **auth none/token**（ROUTE-008/013/017）：deny 401 纯文本 `Unauthorized\n`；
   mutationGate（非 GET/HEAD/OPTIONS 需 launch token；host-gated 标记放行）；
   csrfGuard（POST 必须 application/json；octet-stream 例外路径）；token cookie/
   Bearer/常量时间比较。
3. **信封**（ROUTE-022/023/024）：`Reason {code, error, params}` + dotted 码常量表
   （refuse_common.go 全表）+ writeErr 映射序（OPT-017 单一模块）+ decodeBody
   （64KiB 真实截断、unknown fields 拒绝、`badBody` 不泄露解析细节）。
4. **监听**（ROUTE-020/021）：`RunGracefulHandler` 等价（ReadHeaderTimeout 10s、
   IdleTimeout 120s、drain 10s、shutdown 后 `ErrServerClosed → nil` 语义）+
   `FindPage`（显式 dir 必须 index.html；exe 旁；`../frontend-next/dist`、
   `desktop/frontend-next/dist`；nil = kernel-only）+ withPage 文件优先 + content-type 表。
5. **Broadcaster + SSE**（SSE-001..008 子集，ROUTE-027/028/029 前半）：
   36 kind 的 wire 名称表从 `contract::eventwire` 取（不重写）；droppable 集判定；
   transport-scoped seq；replay 日志 + gap 帧 + watermark；五种订阅模式；
   `GET /events`（帧格式/keepalive 15s `: ping`+watermark/Last-Event-ID）与
   `GET /events/replay`（`{frames, complete, watermark}`）。
6. **路由面**：`/runtimes`(GET)、`/rt/{id}/…` 前缀 + 404 `request.not_found {kind:runtime}`、
   `/tree`(GET)、`/sessions`(GET)、`/history/page`(GET)、`/`（routeDefault：
   无 runtime → 503 `hub.no_runtime_open`）、`/events`、`/events/replay`。
   **不注册**：contract.md §3 裁剪清单的路由（不含假成功）。
7. **golden fixture 回放**：门拒绝三例 + 200 例各一，落 `crates/serve/fixtures/golden/`
   （RUST_MIGRATION.md 的 golden 结构：`__CREDENTIAL__`/`__PORT__` 两占位符）。
   本波次录制不了 Go 真机（record-golden.mjs 需要跑 Go host——若本机能跑则跑，
   不能跑则手写 fixture 并在汇报中标注「未与 Go 实录对拍」）。

## 冻结边界（不得改签名）

contract.md §4 第 2/3/4 条（gate/refuse/broadcaster 三组签名）+ `serve::router`
的组装入口 `build_router(state: RouterState) -> axum::Router`（主 Agent 接线 host 用）。

## 优化条目

OPT-001..005（L0）、OPT-011（门基准）、OPT-012（信封基准）、OPT-013（SSE 写出基准）、
OPT-017（L2 refuse 模块）。每条 L1 先跑直译版基准留数再优化。

## 测试要求

- 单测以 Go 两个测试文件为 oracle 移植（broadcaster/eventstream 逐用例）。
- 门/信封/路由契约测试（逐拒绝码）。
- criterion：`benches/{gate,emit,frames}.rs`。
- 依赖（主 Agent 已预置 workspace）：tokio、axum、bytes、futures-core、tokio-stream。

## 纪律

- 只写 `crates/serve/`；禁改 Cargo.toml / 其他 crate / docs。
- serve 不得被 session/contract/core 反向依赖。
- 并发纪律（硬性）：锁不跨 await；每连接任务可取消（OPT-002）；队列必须有界（OPT-001）。
- 其余同 profile-config 纪律段。
