# profile-config（crates/config 任务书）

## 范围

Go 侧：`internal/contract/config` 中 host 启动所需的子集（全包 29,508 非测试行，本波次只迁子集）。
Rust 侧：`crates/config`（当前 8 行 skeleton）。

## 必读（只读）

- `docs/contracts/config.md`（权威键面/默认值/分层规则）
- `docs/profile-config.md`（本文件）、`docs/contract.md`（切片 S1 + 冻结边界 5）
- `docs/optimization-plan.md`（只看 OPT-010 / OPT-020 + config 的 L1）
- `crates/core/src/lib.rs`（可复用的底层工具）、`crates/contract/src/lib.rs`
- Go 源：`internal/contract/config/`（重点 `config.go`/加载与分层入口、`serve.go` 或
  等价 serve 段文件、desktop 键文件、`storage_roots.go`、`auto workspace` 相关）、
  `reasonix.example.toml`（键面样例）
- host 消费面：`cmd/reasonix-studio-host/main.go`（hostServeConfig）、`startup.go`

## 实现范围（按 contract.md S1）

1. 分层加载：用户层 + 项目层（config.md 的分层/优先级/未知键行为 OPT-010）。
2. `serve` 段：`AuthMode/Token/PasswordHash/BehindProxy` 的解析与归一
   （`NormalizeAuthMode` 语义：lower/trim、空=none、非法=invalid-deny-all）。
3. `host_serve_section()`：类型化清零（HOST-018，OPT-020）。
4. desktop 键：`DesktopTelemetry/DesktopMetrics/DesktopTray`（config.md 对应节）。
5. `AutoWorkspaceDir()`（config.md / host.md §6.1 引用的路径规则）。
6. 顶层 `language` 键（host.md §6.2 的 resolveKernelLanguage 输入）。

不在范围：providers/presets/memory/hooks/appearance 等其余键面（backlog B-016）。

## 冻结边界（不得改签名）

`Config::load(layered_paths) -> Result<Config, ConfigError>`、
`ServeSection::{auth_mode, token, password_hash, behind_proxy}`、
`host_serve_section(&Config) -> ServeSection`（清零后）、
`auto_workspace_dir(&Config) -> PathBuf`、desktop getters。

## 优化条目

OPT-010（L0 未知键矩阵）、OPT-020（L2 清零类型化）、OPT-016（L1 一次 parse 分层 merge，
附 criterion 基准：三层加载优化前后两轮数字）。

## 测试要求

- 单测对照 config.md 用例（默认值矩阵、未知键、分层优先级、auth 归一化）。
- criterion 基准：`benches/load.rs`（三层 load）。
- 新依赖：`toml = "0.8"`（主 Agent 已预置 workspace；crate 内 `{ workspace = true }`）。

## 纪律

- 只写 `crates/config/`；禁改 Cargo.toml / 其他 crate / docs / _go_archive。
- 非 `unwrap/expect`（测试代码允许）；禁 `unsafe`；SQL 无。
- 每模块落盘跑 `cargo check -p reasonix-config`。
- 发现疑似 L3 / 计划外优化：只记录汇报。
- 汇报模板见任务书（状态/完成项/L0/L1 证据/未执行/新建议/契约冲突/依赖/check 结果/需决策）。
