# OPEN_QUESTIONS（迁移窗口期记录）

本文件记录无法从代码直接确定、按「最保守、最贴近现有 Go 行为」原则做了取舍的决策点。
按任务规则：不向使用者提问，逐条保守处置并留痕。全部条目为 2026-11-20 的波次 17
（host 启动切片）迁移窗口内产生。

## 1. 「全部 Go 源 git mv 到 _go_archive/」未执行（偏差声明）

任务规则 2 要求把所有 `*.go`、`go.mod`、`go.sum` 先移入 `_go_archive/`，验证通过后再删除。
本仓库的实际状态与该规则的前提冲突，保守取舍为**暂不移入**，理由：

- Go 内核（`internal/` + `cmd/`，非测试约 42 万行）至今是**唯一可运行的后端**：
  Tauri 壳 spawn 的 sidecar 是 Go 的 `cmd/reasonix-studio-host`（`desktop-tauri` 依赖它启动）。
  账本（`docs/parity/LEDGER.md`）563 行中仍有约 490 条 todo；Rust 内核按仓库既定裁决
  （`docs/RUST_MIGRATION.md` 决策 5、blockers.md B15）在 parity 签收前与 Go 共存，不切换。
- 仓库的多条验证门以 Go 为权威：`tools/wire-parity`（eventwire manifest 的 Go 反射源）、
  `go run ./tools/repolint`、`make check`、golden fixture 录制脚本
  （`scripts/record-golden.mjs` 从 Go host 录制）。移走 Go 立即打断这些门与后续
  波次录制新 fixture 的能力。
- `benchmarks/` 下的 Go 代码是本产品自带的评测夹具（含故意写错的 `go-race-counter`、
  `go-perf-quadratic-index` 等），不是「待迁移的后端代码」，移入归档会破坏评测套件。

**本波次的替代安全网**：全部迁移工作在新分支 `rust-migration` 上进行（自
`feat/tauri-live-events` HEAD `0ade9fc5e` 创建），回滚 = 丢弃分支；这与
`.migration-backup/wave6-src.tgz`（既往波次的备份惯例）同等级。
当账本收敛、parity 签收（B15 解锁）后，由后续波次执行归档与删除。

## 2. `.migration-backup/`（未跟踪目录）

`git status` 显示 `?? .migration-backup/`（内含 wave6-src.tgz，2026-11-17 产物）。
按既往波次惯例这是人工备份产物，不属于工作区改动；未纳入任何提交，保持原样。

## 3. 本波次范围裁剪（账本未收敛的部分）

以下账本区域本波次**未推进**，Go 侧继续作为 live 实现（不属于「实现后删除」范围）：

- `crates/provider`（Go `internal/model`）、`crates/tools`（`internal/tools`）、
  `crates/agent`（`internal/runtime`）、`crates/safety`（`internal/safety`）、
  `crates/ext`（`internal/ext`）、`crates/platform`（`internal/platform`）、
  `crates/tui` / `crates/acp` / `crates/cli`（`internal/frontend` 对应面）：
  trait 骨架冻结，实现留待后续波次（详见 `docs/optimization-backlog.md`）。
- serve 面裁剪：password 鉴权模式（ROUTE-011）、device 门（ROUTE-015/016）、
  events mux（ROUTE-029 后半）、wire log / trajectory（ROUTE-026）、
  Coalesce / CostQuoteSink / NoticeLedger 等 sink 包装（SSE-009..013）。
  host 面裁剪：tray / notify / schedule ticker / update 机制（HOST-015/016/023..031/034..038
  中依赖平台机器的部分）。裁剪清单见 `docs/contract.md` §3。

因此**本波次结束时 Go 源不删除、不归档**；「删除 Go」这一步的完成条件
（账本收敛 + parity 签收 + 真实端到端任务通过新栈）在本波次内未达成，如实记录。

## 4. schedulerun / strip-package-grants 的归属

`HOST-022`（`-strip-package-grants`，Windows-only）依赖 `internal/platform/packagegrant`
（236 行）；`HOST-023`（`schedule exec` 子命令）依赖 Go 侧 `schedulerun` 包。两者都是
平台机器面，本波次未迁移（见 §3）；host 主流程保留它们的 argv 识别槽位与
保守失败语义（识别到参数但实现未迁移时按启动失败 exit 1 处理并记录日志），
不伪造成功。待 `crates/platform` 迁移后接线。

## 5. axum 作为 HTTP 栈的确认

`docs/RUST_MIGRATION.md` 已冻结「async/HTTP/SQLite 栈随 serve 切片（M4）落地」，
未指定具体框架。Go 侧是 `net/http` + Go 1.22 pattern mux（方法+路径+通配段）。
保守选择 **axum 0.8**（匹配 method+path 路由、SSE 支持内建、纯 Rust、MIT/Apache、
满足 deny.toml 政策），理由记入 `docs/tech-choice.md`。SQLite 本波次未触（三个
projection 库都是可丢弃重建物，Go 侧继续持有）。

## 6. 握手 credential 的随机源

Go 侧 `crypto/rand` 32 字节 hex。Rust 侧使用工作区已冻结的 `getrandom`（0.3）
取随机 + 手写 hex 编码，不新增 `hex`/`rand` 依赖（供应链政策：能复用已冻结依赖就不新增）。

## 7. stdin 租约分类的窄化（host）

Go `parentLease` 以 `os.ModeNamedPipe|os.ModeSocket` 判定（host.md §1.5）。
Rust 侧 `#![forbid(unsafe_code)]` 无法调 `GetFileType`/读 handle mode，std 只有
`IsTerminal`。保守近似：**terminal = unsupervised，其余一律 supervised**。
与 Go 的偏差：`host < 文件` 或 `host < /dev/null` 在 Go 是 unsupervised（永久运行），
本实现读到 EOF 即干净退出。生产 spawn（壳以 stdio pipe 启动）两侧行为一致；
偏差见 `apps/reasonix-studio-host/src/lease.rs` 注释。迁移 `crates/platform`
时若有安全 FFI 通道可补精确分类。

## 8. `-instance-id` canonicalize 的窄化（host）

冻结依赖策略（xtask/deps.rs：binaries 只经 assembly）禁止 host 直连 state 的
`canonical_session_path`（Go instanceid 经 platform 间接使用它）。host 内以
`std::fs::canonicalize` 复刻：**marker 文件已存在时**解析符号链接并剥 `\\?\`
前缀，与 Go 的 physical path 等价；home 尚未创建时（marker 不存在）按拼写哈希。
Go 侧是「穿过最近存在祖先」解析。偏差仅在「祖先含符号链接且 home 未创建」时出现；
壳首启即建 home，实际不可达面极窄。待 `crates/platform` 迁移 instanceid 时，
把平台→state 的边放进 ADR-0001 的正式修订再收窄。

