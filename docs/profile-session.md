# profile-session（crates/session 任务书）

## 范围

Go 侧：`internal/session` 的 hub 读侧 + `internal/state/workspacelist`（341 行）
+ `internal/frontend/serve` 中树/会话列表相关的**读模型**部分（hub_tree.go 的读侧、
sessions_list.go）。Rust 侧：`crates/session`（当前 8 行 skeleton）。

## 必读（只读）

- `docs/contracts/http.md` §0（Hub/Runtime 拓扑）、§1.2（/tree、/runtimes）、§7（/sessions 形状）
- `docs/contracts/session-format.md`（§7 列表/§10 读模型——只读，消费不新写）
- `docs/profile-session.md`（本文件）、`docs/contract.md`（切片 S2 + 冻结边界 1）
- `docs/optimization-plan.md`（OPT-006/007/014/018）
- `crates/state/src/lib.rs` 与 `crates/state/src/sessionstore/`（**公开读模型 API——
  listing/order/canonical path 是已完成面，复用，不重写**）
- `crates/contract/src/eventwire.rs`（若树事件需要 wire 类型）
- Go 源：`internal/state/workspacelist/list.go`、`internal/contract/config/storage_roots.go`、
  `internal/frontend/serve/{hub_tree.go 的读侧, sessions_list.go, hub.go 的 findSession/
  publish 部分}`、`hub_projects_benchmark_test.go`

## 实现范围（按 contract.md S2）

1. `workspacelist` 移植：`serve-workspaces.json`（`{"paths":[...],"launch":...}`）读侧 +
   工作区列表（launch 优先于 remembered 顺序）。
2. `WorkspaceTreeSnapshot`：工作区 + 每工作区会话行（`{name, path, title?, turns?,
   current?, unread?, modified}`，http.md §7 /sessions 形状；子代理转写与隐藏恢复副本排除；
   空/不可读目录 → `[]`）。DTO 集中在 `session::model`（OPT-018）。
3. `RuntimeSummary` + `Hub`：空 hub 读侧（`runtimes() -> vec![]`；`open_runtime` 返回
   `HubRefusal::UnsupportedInThisBuild` 的类型化拒绝——写侧机器未迁，不伪造成功）。
4. `BroadcasterHandle` 占位 trait（broadcaster 实体在 serve，session 只声明句柄类型与
   `broadcaster(runtime) -> Option<...>` 签名）。
5. Controller trait 冻结（`SessionApi` 读侧部分签名，写侧方法声明为后续波次锚位）。

## 冻结边界（不得改签名）

contract.md §4 第 1 条的五个方法签名 + `session::model::{WorkspaceTreeSnapshot,
RuntimeSummary, SessionRow}` + `HubRefusal` 枚举。

## 优化条目

OPT-006（L0 容忍语义）、OPT-007（L0 路径规范化复用 state）、OPT-014（L1 树构建基准：
移植 `hub_projects_benchmark_test.go` 的场景为 criterion，优化前后两轮；Go 基线数字
标「未量化」，除非你能本机跑 `go test -bench`）、OPT-018（L2 DTO 集中）。

## 测试要求

- 单测：workspacelist 读侧（launch 优先、不可读容错）、树/会话行形状对照 http.md §7。
- criterion 基准：`benches/tree.rs`（合成 500 会话树）。
- 复用 `crates/state` 测试助手（`crates/testkit` 如可用）。

## 纪律

- 只写 `crates/session/`；禁改 Cargo.toml / 其他 crate / docs。
- session crate 不得依赖 serve/agent/provider（层向：session → state/contract/core）。
- 其余同 profile-config 的纪律段。
