# 波次 18 计划（工具 ABI + 权限判定核）

> 目标：迁移 M3 写侧链条的**两个最深依赖根**——`internal/contract/tool`（工具 ABI，
> 4.9k 行）与 `internal/safety/permission`（判定核，5.7k 行，含 48 行 permrule 语法）。
> 这两者是 builtin 工具（TOOL-024..035）与 agent 循环（B-008）的共同前置；完成即解锁
> 下一波 builtin 工具。波次 17 已完成的 serve/host/config/session 不动。

## 切片边界

### Agent-ToolAbi（crates/tools）

Go `internal/contract/tool` 全量接口面 → `crates/tools::abi`（或按模块拆 tools::registry 等）：
- TOOL-001：`Tool` trait（Name/Description/Schema/Execute/ReadOnly）+ 可选能力 trait
  （ContextualTool/ContextualReasoner/Previewer/WritePathResolver/ReadPathResolver/
  ReadRefusalFormatter/SequentialTool/TargetAccessCheck）——Go 的 type-assertion 可选
  接口在 Rust 用「能力 trait + downcast/dyn 兼容查询」表达，形状由 Agent 定，冻结的
  是**语义**（批量判定只看 ReadOnly、Preview 走 Execute 同一解析路径等）
- TOOL-002..003：per-run Registry（restriction/replace-prefix/withdrawn/ContractEntry
  快照 + provenance）
- TOOL-004..005：argument envelope 解包、ShellExecution 元数据
- TOOL-011：Refusal 身份/补救 + blocked errors（类型化，非字符串匹配——仓库铁律）
- TOOL-008：Invoker + 嵌套调用（深度上限语义）
- 其余行（MCP 绑定 006、Roots 007、peer/goal/recall/settings/elicit/paidsearch/
  configwrite/compress/video/annotations/reach/transcript/background）若依赖未迁移
  机器则声明 trait 锚位 + todo，不伪造

对照：`docs/contracts/tools.md` §1.1-§1.16（已冻结）+ Go 源 `internal/contract/tool/*.go`
（含 *_test.go 的 oracle：contract_test/invoke_test/goal_test/arguments_test 等逐用例）。

### Agent-Permission（crates/safety）

Go `internal/safety/permission` 判定核 → `crates/safety::permission`：
- permrule 语法（48 行，`crates/safety::permission::permrule` 或独立小模块）：
  `Rule{Tool,Subject,Literal}` + Parse 三形态（"Tool"、"Tool(glob)"、legacy "Tool=literal"）
- Decision 三态（Allow/Ask/Deny）+ ParseDecision（unknown/empty→Ask 保守）
- Policy：纯规则求值（无 I/O）——优先级/首匹配/写篱笆（write_fence）/subject 匹配
  （glob vs literal）/deny 规则组/legacy bare PowerShell deny cmdlet 表
- Verdict 类型（verdict.go）与 Gate 组合形状（Policy + Approver trait 锚位）
- bash_approval/bash_decompose/bash_readonly/bash_redirect 等 shell 判定**若仅依赖
  core::shellparse 已迁面则一并迁**；依赖超出（mvdan AST 全量语义）的部分声明锚位
- 对照：Go 侧 permission_test/bash_approval_test/write_fence_test/verdict_test 等
  oracle 逐用例移植；敏感路径纪律（REASONIX.md：safety/permission/** 与 shellsafe/**
  是 declared sensitive——改动须最小、逐行为对照，不做超范围重构）

## 冻结边界（两 Agent 共同）

- crates/tools 与 crates/safety 的对外签名在本文件 + 各自 profile 中冻结；
  互相不得依赖（tools 可依赖 safety，safety 不得依赖 tools——xtask policy 线）
- 不改根 Cargo.toml（workspace 依赖已够：serde/serde_json/thiserror/tokio/tracing/
  regex/tempfile/criterion；若需 shell 语义扩展，core::shellparse 只读复用）
- 不触波次 17 已完成面（serve/session/config/host/assembly）

## 验收

- 两 crate 测试全绿 + clippy -D warnings + fmt + xtask deps/parity
- TOOL-001..005/008/011 与 permission 相关账本行翻 done/wip（附测试名）
- docs/optimization-review.md 追加波次 18 节（同一审查代理或主 Agent 复核）
- docs/migration-report.md 追加波次 18 章
