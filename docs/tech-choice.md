# 技术选型（tech-choice.md）

任务模板中的「gorm → sqlx 手写 SQL」条目对本仓库不适用：Go 内核**不使用 ORM**；
业务真相是 JSONL 会话日志（`crates/state` 已完成移植），三个 SQLite 库是可丢弃重建的
投影（`docs/RUST_MIGRATION.md` 决策 3），且本波次不触任何数据库。以下记录本波次
实际做出的栈内选择与理由。

## 1. HTTP 栈：axum 0.8（+ tokio 1.x）

- Go 侧是 `net/http` + Go 1.22 pattern mux（`METHOD /path` + `{id}` 通配段）。
  axum 0.8 的 method+path 路由与 `{param}` 语法与之同构，`404/405` 显式可控，
  SSE（`axum::response::sse` 或手写 body 流）可达契约的逐帧 flush 语义。
- 纯 Rust、MIT/Apache，满足 `deny.toml`（licenses allowlist + wildcard ban +
  unknown-source deny）。
- 备选 hyper 直接裸写被否：路由矩阵 420 条，手写 dispatch 的复杂度与漂移风险更高；
  actix 被否（actor 模型对 port 语义无增益，且 tokio 生态一致性优先）。

## 2. SSE 传输：axum body 流 + 手写帧

契约要求逐帧 flush、`: ping` 注释 + `stream_watermark` 真帧、`id:` 仅编号帧
（http.md §5）。`axum::response::sse::Sse` 的 Event API 无法表达「无 id 的帧」与
注释+数据混合帧的全部组合，因此 Agent-Serve 手写 body 流
（`futures_core::Stream` + 有界 mpsc 源），不使用内建 Sse 类型。

## 3. 配置解析：toml 0.8

对应 Go `BurntSushi/toml`。分层加载/合并语义由 crate 内实现（一次 parse 成
`toml::Value` 树再 merge，见 optimization-plan OPT-016），不引 `config`/`figment`
——它们的分层模型与 config.md 冻结的分层规则不一致，强行映射反而引入漂移。

## 4. 随机数：getrandom 0.3（已在工作区）

握手 credential（32 字节 CSPRNG hex）。`getrandom` 已在契约清单依赖里；
不新增 `rand`/`hex` 依赖（hex 手写 16 行，供应链面最小化）。

## 5. 常量时间比较：手写 XOR 折叠

Go `subtle.ConstantTimeCompare` 的等价物用 ~8 行手写实现（长度无关 + XOR 折叠），
不引 `subtle` crate。由单测钉住「长度不同也恒时」。

## 6. 基准：criterion 0.5（dev-dependency）

任务模板指定。热点路径：门检查（OPT-011）、事件信封序列化（OPT-012）、
SSE 写出（OPT-013）、树构建（OPT-014，对照 Go `hub_projects_benchmark_test.go`）。

## 7. 不引入的东西

- 不引 `tower-http`：仅需 catch-panic 一个能力，crate 内 ~10 行中间件足够
  （OPT-003），避免为一个能力拉全家桶。
- 不引 `sqlx`/`rusqlite`：本波次零 SQL 面；三个投影库仍归 Go（可重建物，见上）。
- 不引 `async-trait`：edition 2024 + Rust 1.99 原生 async trait 可用。
