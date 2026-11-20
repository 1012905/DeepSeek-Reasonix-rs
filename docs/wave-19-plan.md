# 波次 19 计划（bash AST 分析引擎 + shellsafe 判定表）

> 目标：为波次 18 留下的 `BashAnalysis` 引擎 seam 落地 Rust 侧 AST 级分析器，
> 翻开 SAFETY-008/009 的 10 个锚位；随后（若本波次余量允许）迁移 read-only
> builtin 工具切片（TOOL-028 的 read_file/glob/ls + TOOL-050 confine）。

## 切片 1：Agent-BashEngine（crates/core + crates/safety）

Go 侧三个依赖根：
- `internal/base/shellparse/bash.go`（768 行）：ParseBash/StaticFields/AnalyzeApprovalFeatures/
  ContainsUnquotedGlob/CanMaskEarlierFailure/MasksOnlyInsideFinalPipeline/SplitTopLevel/
  StaticWord/IsAssignment/WordBase/CompoundLeaves/StdinHereDocPrograms/StdinHereDocArgv
  + heredoc.go/pipeline.go/wrapper.go（PeelWrappers）/exit_status.go/delete.go
- `internal/safety/shellsafe/`（约 1.6k 行）：ReadOnlyCommands/ReadOnlyPrefixes 表 +
  ClassifyReadOnlyCommand/Fields/ArgsMakeReadOnlyCommandWrite/OperandsNameNetworkPath/
  NormalizeBashSafeRedirectsForMatch/GitContentReadTargets/AnalyzeRecursiveDelete/
  UnwrapDeleteCommand/ExecutableBase
- `internal/safety/permission/bash_readonly.go`（172 行）：BashReadOnlyProof + planmode.Proof

Rust 实现路线（Agent 裁决并写进汇报）：
- **不得引入 unsafe**；纯 Rust 实现。可选路线：(a) 扩展 `crates/core::shellparse` 的
  字节级 bash 词法器覆盖上述全部消费面（波次 18 已有 static-command 子集与回归测试）；
  (b) 评估移植现成 Rust bash 解析器（crates.io 搜索；受 deny.toml 供应链政策约束：
  纯 Rust + MIT/Apache/ISC，未知来源拒绝）。**默认 (a)**，(b) 只有找到成熟纯 Rust crate
  才可提案（写入汇报，主 Agent 批）。
- 语义锚定：Go 侧三个包的 ***_test.go 全部用例**是 oracle（shellparse 的
  bash_test/pipeline_test/wrapper_test/heredoc_scope_test/delete_test/delete_powershell_test/
  exit_status_test/mask_test + shellsafe 各 test + permission 的
  bash_approval_test/bash_readonly_test/bash_decompose_test）。
- 落盘：分析器主体进 `crates/core/src/shellparse/`（扩展现有模块，保持其已冻结的
  static-command API 不破坏——既有测试必须继续绿）；shellsafe 表进
  `crates/safety/src/shellsafe.rs`（或模块）；`permission::bash` 的
  `set_bash_analysis` 装入真引擎，**SAFETY-008/009 的 8+2 锚位测试去 #[ignore]
  必须全绿**——这是本波次的验收硬门。
- 敏感路径纪律：shellsafe/permission 是 declared sensitive（REASONIX.md）——
  只做逐行为对照移植，无超范围重构；任何不确定行为 fail-closed 并在汇报中点名。

## 切片 2（余量允许时）：Agent-ReadonlyTools（crates/tools）

TOOL-028 读侧 + TOOL-050：Go `internal/tools/builtin/{read_file,glob,ls,grep,confine}`.
go + 对应 test → `crates/tools::builtin`。依赖切片 1 的引擎与波次 18 的 ABI。
若切片 1 未收口则本切片不开（串行）。

## 验收

- 全部 Go oracle 用例移植绿；锚位测试去 ignore 后全绿；51+ 套件回归绿；
  clippy -D warnings / fmt / xtask 双门绿。
- 账本：SAFETY-008/009 翻 done；TOOL-025（bash shell tool）/TOOL-028（读侧）/
  TOOL-050（confine）按实际完成面翻行。
- 审查追加 + 报告追加（同波次 18 格式）。
