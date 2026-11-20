---
owner: "@SivanCola"
backup: "@esengine"
status: active
reviewed: 2026-11-18
---

# 迁移状态（断点续跑文件）

续跑时先读这一份。每个波次结束都要更新它；不许让它描述的树比真实的树旧。

最后更新：2026-11-20（波次 17–19：Rust 内核 host 启动切片 + 工具 ABI/权限核 + bash AST 引擎/只读工具——config/session/serve 三 crate
落地 + apps/reasonix-studio-host 端到端接线；账本 done 69→103；release 冒烟 8 契约行为
全过；分支 `rust-migration`）。

## 波次 18–19 改了什么（写侧链条前半）

- **波次 18**（`docs/wave-18-plan.md`）：`crates/tools`（contract/tool ABI：Tool trait +
  CapabilitySet/Registry/argument envelope/Refusal/NestedCallID/schema canonicalize，
  63 测试）与 `crates/safety::permission`（permrule/Policy/Gate/write fence/session
  grants/inline-code，57 测试 + 8 锚位）。账本 done 103→117。审查放行（零 MAJOR）。
- **波次 19**（`docs/wave-19-plan.md`）：
  - 切片 1：bash AST 引擎——词法核携带结构（joins/spans/stdin-heredoc/negation），
    `shellparse_analysis`（SplitTopLevel/CompoundLeaves/PeelWrappers/ApprovalFeatures/
    Glob/Mask），`safety::shellsafe` 判定表，network AST 化（redirect 原文拼写归一、
    word-skeleton 网络操作数）。10 锚位测试去 ignore 全绿。SAFETY-008/009 done。
  - 切片 2：只读 builtin 工具——confine 读半/secretpaths/network/pathgo/globengine
    （Go filepath+doublestar 行为级）/gitignore/walk 剪枝/read_file 窗口/ls/grep 两相。
    tools 150 测试绿。TOOL-028/050/027/056 wip（裁剪面：code_index/rg 子进程/confine
    写侧/PathResolver）。
- 账本：done **119**、wip 29、todo 424（起点 done 69）。
- 分支 `rust-migration` 累计 25 提交；workspace 55 套件全绿；clippy/fmt/xtask 全 0。
- 裁剪链下一环：builtin 写类（TOOL-029）→ jobs（TOOL-046..048）→ confine 写侧 →
  rg 子进程 → agent 循环（B-008）→ serve 写路由。

## 波次 17 改了什么（Rust 内核 host 启动切片）

用户指令：把 Go 后端原地迁移为等价 Rust 内核（契约不变 + 迁移窗口优化），Go 源在验证
通过后删除。与仓库既定迁移（RUST_MIGRATION.md、账本、B15 cutover 门槛）对齐后，本波次
推进账本 M4 serve 切片的读侧 + host 启动面；**Go 源未删除未归档**（账本未收敛，
B15 仍然成立——`OPEN_QUESTIONS.md` §1 有完整论证）。

- **crates/config**（S1）：分层加载（未知键透传/坏层丢弃/警告文本逐字对齐 Go）、
  serve 段 auth 归一化、host_serve_section 清零（HOST-018）、desktop 键、
  auto workspace 根链、language、user config 选择链（PATHS-005 主体）。41 测试。
- **crates/session**（S2）：workspacelist 读侧、树分组、原生会话 listing
  （/sessions 行形状）、title 缓存、冻结 Hub 读侧面。51 测试。
- **crates/serve**（S3）：loopback 门（golden 与 Go 实录逐字节对拍）、auth none/token、
  Reason 信封 + refuse_common 19 码、decodeBody 真实截断、优雅监听（10s/120s/10s）、
  FindPage（BREAK-002 记录契约文档路径陈旧）、Broadcaster+SSE 全机件（droppable/
  transport-scoped seq/8MiB 回放/4MiB 逐级摘除/五订阅/gap/watermark/keepalive）、
  hub 级路由（/runtimes /tree /sessions /rt/{id} routeDefault）。
  85 测试（含 3 golden 重放）。
- **apps/reasonix-studio-host + crates/assembly::host**（S4）：按 ADR-0001
  （binaries→assembly；config 经 serve frontend 到达——ADR key choice 3 的既有路径，
  未改冻结策略）完成端到端接线：argv 优先级/flag 集/hex 凭据/握手行/act 行/12 相位日志/
  租约/instance-id/优雅排空。release 冒烟：无 cookie 403 unauthorized、错 Host 403
  host_rejected、无 Origin 变更 403 origin_rejected、带 cookie GET / 200 页面、
  /tree //sessions 200、未知路由 503 hub.no_runtime_open、租约 EOF → exit 0。
- **优化**（详见 docs/optimization-plan.md 与各子 Agent 证据）：OPT-001..005/008..020
  逐条执行或按证据否决（OPT-016 的 Value 树 merge 方案实测负收益，保留 direct 解码，
  merge 实现留 benches 复核）；criterion 基准落盘（gate 40ns vs 直译 266ns；
  帧 122ns vs 463ns；树 100 根 28-34ms vs Go 基线 89.1ms，口径差异见报告）。
- **账本**：done 69→103、wip 23（CONFIG/PATHS/ROUTE/SSE/HOST 各行证据齐附）。
- **并行会话事件**：工作树中出现另一会话的未提交改动（desktop-tauri remote/share +
  Go share 面），一次 `git add -A` 曾卷入；已 `git reset --soft` 拆分并原样保留，
  详见 OPEN_QUESTIONS §9；此后只用显式路径 add。

**波次 17 验证（主 Agent 重跑非子 Agent 自报）**：

```
$ cargo test --workspace                     # 46 套件全绿（config 41/session 51/serve 85/
                                             #   assembly 10/host 14+4）
$ cargo clippy --workspace --all-targets -- -D warnings   # 0
$ cargo fmt --all --check                    # 0
$ cargo xtask deps --check                   # OK（14 internal edges）
$ cargo xtask parity --check                 # OK（563 行：done 103 / wip 23 / todo 437）
$ cargo build --release -p reasonix-studio-host   # 0
$ release 冒烟（curl 走门）                  # 8 契约行为全过（见上）
```

cargo-deny 本机未装仍 skipped（B15 记录不变）；oha/wrk 未装，冒烟以 curl 代。
裁剪面（password/device 门/events mux/wire log/写侧路由/provider/tools/agent/safety/
ext/platform/tui/acp/cli）记录于 `docs/contract.md` §3 与 `docs/optimization-backlog.md`。

> 本文每个数字都来自下方「已验证基线」里的命令。没有命令支撑的判断标「未验证」。
> 波次 5 的 284 文件 / 2140 测试 / 16 命令 / 129 孤儿已被本波次实测取代。
> 「A 与主干的实测 diff」「B 的规模」「组件普查」三节是波次 5 的历史快照，未在本波次重跑。

## 波次 8 改了什么（采纳上游 Rust 内核，D14）

用户裁决采纳 B 的**上游最新版**内核重写（Gitee `rikkarikka/reasonix-studio-rs` main，
HEAD `9b3d7ba`，M2），取代 D3 的「不合入」。这与主干自身 `RUST_MIGRATION.md` 的既定
方向一致；A 谱系的停放骨架（9 个 1 行 stub ＋ contract 2016 行）由 B 上游取代。

- **来源**：clone 到 `Downloads/reasonix-studio-rs-main`（depth 40）。上游 87 个 `.rs`／
  33 751 行；账本 563 行＝69 done／1 wip／493 todo。质量集中在 state（22 747 行）/
  contract（4 807）/core（4 641）；agent/tools/provider/serve/safety 仍 8 行 skeleton。
- **合并（按模块择优，非整目录覆盖）**：B 的 `crates/`（17）＋`apps/`（reasonix,
  reasonix-studio-host）＋`xtask/`＋根 Cargo.toml/lock＋clippy/deny/rustfmt/rust-toolchain
  ＋`.cargo/config.toml` 进 trunk。**保留 A `contract` 的 eventwire（＋golden manifest＋
  parity 测试）/pricing/agentgraph/delivery/surfacespec** 并入 B 的 `reasonix-contract`
  （B 账本 SSE-004 eventwire 仍 todo，A golden 门领先；模块名零冲突，A 仅 serde/std、无 unsafe）。
  根 Cargo.toml 加 `exclude = ["desktop-tauri/src-tauri"]`，壳保持独立工作区。
- **文档**：B 的 `docs/contracts`（14 份冻结契约）、`docs/parity/LEDGER.md`（＋BREAKS）、
  `docs/adr/0001` 采纳为 trunk live 文档；B 的 AGENTS/README 存为 `docs/kernel-rs/` provenance。
  trunk 根 AGENTS.md/README.md/REASONIX.md（Go standing instructions）未覆盖。
  `RUST_MIGRATION.md` 的 layer mapping 与依赖段更新为 B 布局（base→core、model→provider、
  runtime→agent、boot→assembly、kernel bin→apps/）。B6 归档冲突就此消解（采纳即 live，不归档）。
- **Go/壳/Vue 保持 live**：Go host 仍是实际后端（Rust `apps/reasonix-studio-host` 是 stub，
  exit 2，HOST 行在 M4）。无 cutover（B15）。
- **网络**：crates.io/rsproxy CDN 本机 DNS 间歇失败 → `~/.cargo/config.toml` 配 USTC sparse
  镜像（机器级，不提交）；仓库 `.cargo/config.toml` 只留 xtask alias。

**波次 8 验证（本机实跑，USTC 镜像）**：

```
$ cargo test --workspace --all-features      # trunk 根内核工作区
572 passed; 0 failed（含 A 保留的 eventwire_parity golden 套件）; EXIT 0
$ cargo fmt --all --check                    # FMT 0
$ cargo clippy --workspace --all-targets --all-features -- -D warnings   # 0 warning; EXIT 0
$ cargo xtask parity --check                 # OK (563 rows; todo 493, wip 1, done 69)
$ cargo xtask deps --check                   # OK (19 manifests, 6 internal edges)
$ go build ./...                             # 0；internal/ cmd/ go.mod 零改动
$ cd desktop-tauri/src-tauri && cargo check  # 0（1.99.1 pin 下重编译 3m59s）
```

`cargo-deny/nextest/machete` 本机未装 → skipped（B15）。Vue 前端本波次未改，
基线仍 294 文件 / 2183 测试（波次 7）。

## 波次 9 改了什么（前端有界收口：C5 / B4 / B2，B9 派发）

Go/Rust 内核本波次零改动（用户指令：无需更改的 Go 不动，维持部分重构 Rust+Go 共存）。
仅前端有界收口与对等门恢复：

- **C5（done）**：`ui/App.vue` 的 `openFiles` 补齐 React 基准的两个副作用——无活动 pane
  时 `error` 提示「先打开会话」、有覆盖的设置 sheet 先 `closeSettings()` 再脉冲 files bar。
  新增 `ui/e2e/filesBar.interaction.test.ts`（2 测试，真实挂载 App 证明两条）。Vue 无
  React 的 `roomGaveUp("scene")` 第三副作用（该子系统 Vue 不存在），只实现基准有的两条。
- **B4（kind 门 done，type-witness 对等记录待办）**：纠正旧记录——HEAD 的
  `wire.parity.test.ts` 测的是 `./wire` ↔ `crates/contract/src/eventwire_manifest.json`
  （Go 反射权威，67 类型，与 Rust `eventwire_parity.rs` 同源），**非** kernel.ts/HubPort。
  worktree `wire.ts` 已重构（vs HEAD 500+/598−：重命名 ShellExecution→Execution 等、拆兄弟
  文件、删 receipt/recovery/video/workspace/rate/valuation 类型），manifest 仍声明旧名，故
  HEAD 898 行 witness 无法过 vue-tsc。**有界落地**：`wire.ts` 增 `KINDS` const 元组、`Kind`
  由它派生（单一真相）；测试缩为 **kind-set 双向对等门**（36↔36，kind 是稳定路由面）。
  完整 type-witness 对等需 deliberate 契约 reconciliation（Vue 名↔Go 名映射＋被删类型取舍），
  属大规模重构之外，记录于 B4 跳过。
- **B2（裁决跳过，不改 mock）**：复核「恢复未实测」不成立——`state/restore.test.ts` 已用
  连贯非空 HistoryMessage fixture（合成 port）实测 restoreSession+fromHistory+reducer；
  分页由 history_pages/useHistoryPages 测试覆盖；Pane→history 接线由 App.port.test.ts:77 断言。
  `MockPort.history()=[]` 是有意 fixture 语义且 21 个 UI 测试依赖它，播种会破坏邻域、增益已被
  上述覆盖提供。真实浏览器重开是 B1 阻塞，与 mock 无关。纠正记录于 B2。
- **D5（blocked B16）**：pinia 死依赖删除触发 pnpm-lock↔package.json 长期脱钩的全量重解析
  （4663+/676−），已 net-zero 回退（lock 还原 HEAD、package.json 复原），记录 B16 待专门 resync。
- **B9（done，子 Agent 执行＋主 Agent 收口重跑）**：PDF/Office 查看器移植——`PdfPreview.vue`
  （133 行，pdfjs canvas/TextLayer＋页码/缩放/适应宽度）、`OfficePreview.vue`（83 行，docx/xlsx
  sandbox iframe＋工作表 tablist）对齐 React 基准；`DocumentPreview.vue` 加 pdf/office 支路＋
  基准 office-off 设置门；样式并入 `src/design/document.css`；i18n 键补齐；`orphan.test.ts`
  移除 officeSheet/officeWord（查看器 reached）。deps 与解析助手早已在，pdfAssets() 插件构建期
  emit dist/pdfjs 资产。组件级有界，不动 App/Pane。详见 blockers.md B9（已解决）。

**波次 9 验证（本机实跑，主 Agent 重跑非子 Agent 自报）**：

```
$ cd desktop-tauri && ./node_modules/.bin/vue-tsc -b --force   # EXIT 0
$ ./node_modules/.bin/vitest run                               # 298 文件 / 2196 测试全绿; EXIT 0
$ ./node_modules/.bin/vite build                               # EXIT 0（emit dist/pdfjs/{cmaps,standard_fonts,wasm}＋worker）
$ ./node_modules/.bin/vitest run src/ui/document/ src/orphan.test.ts src/i18n/msg.test.ts  # 7 文件/67 测试绿
```

基线由波次 7 的 294/2183 增至 **298 文件 / 2196 测试**（＋filesBar 2、＋wire kind-set 1、
＋B9 查看器 2 文件/10 测试）。孤儿普查：officeSheet/officeWord 转 reached（B9），deliveries/blocks
仍孤儿（另一条 delivery-card 链）。Go `go build`/Rust `cargo` 门本波次未触发（零改动）。

## 波次 10 改了什么（B14 会话菜单可达切片）

Go/Rust 内核本波次零改动（用户指令：无需更改的 Go 不动，维持部分重构 Rust+Go 共存，
不推进 cutover）。仅 Vue 前端会话菜单的**可达完整切片**，子 Agent 执行组件＋主 Agent
接线/裁决/收口重跑：

- **B14 可达切片（done）**：`sidebar/SessionMenu.vue` 由 rename/archive/delete 三项扩为
  四组菜单，新增 **pin / move(up·down) / auto-name / mark-read / copy-info / export(md·json)**。
  所有权镜像 React `Workspaces.tsx`：这些变更由 `Sidebar.vue` 自跑（`hub` + `reload` +
  `onError`），`App.vue` 仅供 `reload: refresh`（汇合文件最小改动）。四个原孤儿
  `sessionExport.ts`、`sessionExportMarkdown.ts`、`sessionInfo.ts`、`useSessionMove.ts`
  被接线到达，从 `KNOWN_ORPHANS` 移除（debt **92→88**）。CSS/icon/i18n 早已齐，无新增。
- **take 顺序根因修复（主 Agent 裁决）**：`SessionMenu.vue` 的 `take` 原先先
  `emit("close")` 再跑动作；Sidebar 的 `@close` 处理器清空 `menuAt`，致动作读到空目标
  ——rename/archive/delete 同样受影响的**潜在 bug**（此前无侧栏级菜单测试，故未暴露）。
  改为**先跑动作再 announce close**，对齐 React（父级先捕获 session 再关闭）。这是 owning
  layer（组件的 take 协议）的一处修复，惠及所有菜单项，非症状处补丁。
- **仍缺（精确架构原因，诚实缺席而非半接线）**：`onPause/onClose/onBranch/archive-live-disable`
  需 Vue 没有的**每会话 mid-turn/live 状态**（`workspaceTree.ts:41-43` 刻意不携带），
  branch 另需 `openPane`（B12）；`kept-versions` 需 versions+copies 分散 UI；`onMoveTo`
  需 Vue 没有的 `useSessionMoveTo` 助手＋项目选择对话框。组件以可选回调门控（缺则不画）。
  详见 blockers.md B14（已收窄为 partial）。

**波次 10 验证（本机实跑，主 Agent 重跑非子 Agent 自报）**：

```
$ cd desktop-tauri && ./node_modules/.bin/vue-tsc -b --force   # EXIT 0
$ ./node_modules/.bin/vitest run                               # 300 文件 / 2216 测试全绿; EXIT 0
$ ./node_modules/.bin/vite build                               # EXIT 0（chunk-size 警告为既有，非错误）
$ ./node_modules/.bin/vitest run src/ui/sidebar/SessionMenu.interaction.test.ts \
    src/ui/e2e/sidebarSessionMenu.interaction.test.ts \
    src/ui/e2e/sidebarReachable.interaction.test.ts src/orphan.test.ts   # 4 文件 / 29 测试绿
$ grep -c '^  \["src/' src/orphan.test.ts                      # 88
```

基线由波次 9 的 298/2196 增至 **300 文件 / 2216 测试**（＋SessionMenu 组件测 1 文件/11 测试、
＋sidebarSessionMenu 邻域测 1 文件/9 测试）。邻域测真实挂载 Sidebar＋MockHub，逐项证明
pin/mark-read/move/auto-name/export/archive 路由到对应 hub 变更＋reload＋菜单关闭，
及可选回调门控（无 rename 则不画、unread 才有 mark-read）。Go/Rust 门本波次未触发（零改动）。

## 波次 11 改了什么（前端子系统收口：B12-core/B11-Files/B14-runtime/B10b ＋ D15 裁决）

Go/Rust 内核本波次零改动（用户指令：无需更改的 Go 不动，维持部分重构 Rust+Go 共存，
不推进 cutover）。仅 Vue 前端把剩余子系统驱动到终态——能忠实移植的整条移植并接线，
结构性不可达或依赖未移植子系统的余项有界裁决（D15），非半措施。子 Agent 执行组件＋
主 Agent 接线/裁决/收口重跑：

- **B12-core pane-opening（done）**：`App.vue` 引入每-pane `viewRequests =
  ref<Record<string,{to,n}>>`（:135）＋ `openPane`（:332，busy 守卫 hub.open→refresh→active）
  ＋ `requestView/showView`（:395-399），传给当前 pane（:651）并被 `useBranchSession`（:345）
  消费。**sidebar-open-to-flow 内联**在 `openSession`（:318-328，重点击已开会话即
  `requestView(id,"flow")`），按 D13 `sidebaropen.ts` 保留孤儿（功能已在）。并发开 ticket
  排序（`sessionswitch.ts`）未移植——Vue 串行开 pane，无消费点，保留孤儿（D15 暂缓）。
- **B11-Files rail（done）**：`Metrics.vue` ORDER 增 `files` 段、`import Files`、渲染
  `<Files :changes :tree :yolo>`（:89）；`Pane.vue` 派生 `rail = railOf(items,executions,
  subagentPhase)`（:136，`panels/derive`）并把 rail.changes/tree/yolo 喂 analysis 视图的
  Metrics（:406-408）。`panels/Files.vue`/`panels/derive.ts` 转 reached，移出 KNOWN_ORPHANS。
  Plan 面板与 conversation-files 链未移植（D15 暂缓）。
- **B14 运行时动作（done）**：B12 的 `runs = ref<Record<string,{live}>>` 供 Sidebar
  `menuLive`，`SessionMenu.vue` 以 `held && live` 画 pause/branch、close 在 live 时 disabled
  附「先暂停」说明；Sidebar 接 `:on-pause/:on-close-conversation/:on-branch`（:489-491）
  经 take 路由到 onPauseSession/onCloseSession/onBranchSession，branch 经 useBranchSession→
  openPane 落源 root 旁。kept-versions/onMoveTo 仍缺（依赖未移植子系统，D15 暂缓）。
- **B10b agent-runs 工作台（done）**：新增 `ui/useAgentRequest.ts`（open/openJob/consumed，
  shallowRef 按身份清除）＋ `ui/AgentRunView.vue`（轮询 subagentTranscript、任务抬头、
  运行态/停止/只读注记）；`useAgentTabs`（agent/job Surface）被 `WorkbenchPanel.vue` 消费
  （surfaces 增 agents、closeSurface 处理 agent/job、渲染 AgentRunView）；`Pane.vue`
  `provide(AgentRunOpener, agentRuns.open)`、reveal=openColumn("files")、传 agentRequest。
  SubagentGroup AgentRow 打开按钮经 inject 到达，live Transcript 已用 workLanes（B10a）。
  `useAgentTabs` 转 reached（debt 84→83）。组件测 17＋工作台接线测 1。B10c job 查看器/
  B10d onShowCall（Transcript focus inert，返回按钮诚实缺席）/openCards 有界裁决（D15）。
- **B13 工作台浏览器标签改名（adjudicated out-of-scope，D15）**：浏览器走停靠列（D6/D12）
  不进工作台标签条，改名 browser-only 结构性不可达，`useTabNames.ts` 按 D13 保留孤儿。

**波次 11 验证（本机实跑，主 Agent 重跑非子 Agent 自报）**：

```
$ cd desktop-tauri && ./node_modules/.bin/vue-tsc -b --force   # EXIT 0
$ ./node_modules/.bin/vitest run                               # 304 文件 / 2250 测试全绿; EXIT 0
$ ./node_modules/.bin/vite build                               # EXIT 0（3292 模块；chunk-size 警告既有）
$ ./node_modules/.bin/vitest run src/ui/workbench/WorkbenchPanel.interaction.test.ts \
    src/ui/agent-run-tab.interaction.test.ts src/ui/useAgentRequest.test.ts \
    src/orphan.test.ts src/ui/Transcript.subagents.test.ts     # 5 文件 / 30 测试绿
$ grep -c '^  \["src/' src/orphan.test.ts                      # 83
```

基线由波次 10 的 300/2216 增至 **304 文件 / 2250 测试**（＋B10b agent-run-tab 9 +
useAgentRequest 8 两文件、WorkbenchPanel.interaction +1 接线测，＋B12/B11/B14-runtime
邻域测）。孤儿债务 **83**（波次 10 记录 88，本波次实测 83；useBranchSession、panels/Files、
panels/derive、useAgentTabs 等转 reached 移出 KNOWN_ORPHANS）。Go/Rust 门本波次未触发（零改动）。

**波次 11 补记（依赖重同步，B16 resolved）**：用户实际运行 `pnpm -C desktop-tauri tauri dev`
触发 B16 —— pnpm 12.10.1 不再读 package.json 的 `pnpm` 字段。`pnpm.overrides` 移入
`pnpm-workspace.yaml` 顶层 `overrides:`，`pnpm install` 重同步（lock **+4286/−206**；
3 个直接依赖在 `^` 范围内前移：dompurify 3.4.18、user-event 14.6.12、test-utils 2.6.0；
无大版本变更）。`vendor-windows-names` 守卫改读 workspace 的结构化条目。回归：
vue-tsc 0；全量 **304 文件 / 2250 测试绿**；`tauri --version` → 2.12.4 EXIT 0。
`tauri dev` 在本代理进程内仍卡 WebView2（B1：0x800700AA；换空数据目录后 0x8000FFFF
且新目录未创建）——用户 04:58 的自跑日志没有 `startup failed` 行，疑为代理宿主限制，
待其终端复跑确认。另把 `Pane.turn.interaction` 的 20s 等待放宽到 60s 以吸收满负载偶发。

## 波次 14 改了什么（P2 + P3 执行）

用户裁决「原来的账号部分一起去掉；版本更新转为现在的 reasonix-rs 项目」。P4 Worker 骨架已在
波次 13 提交；本波次完成 P2（旧账号面退役）与 P3（更新通道改指本仓库 Releases），详见 D18/D19。
执行方式：三路并行代理（P2-Go / P2-Vue / P3），其中两路因推理配额中断，主 Agent 接手收口。

- **P2**：Vue 删账号/市场发布/云备份/手机/云远程/反馈榜（反馈对话框保留）；TraceExport 迁 storage；
  i18n 孤儿键批删 237；端口与 wire-parity 同步摘除；serve 删 account/backup/cloud/market-publish/
  phones 路由（−2709 行），`notify.Phone` 随唯一调用者删除；DeviceShare LAN 门保留。
- **P3**：
  - 目录 → `github.com/1012905/DeepSeek-Reasonix-rs/releases/latest/download/versions.json`。
  - BundleID/polkit → `com.esengine.reasonix-studio`（instanceid 稳定）。
  - 新 `release-studio-tauri.yml`（三平台、`bundle.resources` 携 host+page、minisign 旧钥、
    catalog 挂 latest 资产）。
  - `withdraw-studio-version.yml` 改操作 release 资产；backfill 脚本退役。

**波次 14 验证（真实执行）**：

```
$ vue-tsc -b --force                                 # 0
$ vitest run                                         # 297 文件 / 2170 测试全绿
$ vite build                                         # 成功
$ go build ./... && go vet ./...                     # 0
$ golangci-lint v2.12.2（GOTOOLCHAIN=go1.26.7）      # 0 issues
$ go run ./tools/repolint                            # clean (3553 baselined)
$ go test serve/cmd/account/remotecloud/notify/update/instanceid/studio-manifest  # 全绿
$ node --test scripts/*.test.mjs（notes/release/withdraw）  # 12 + 6 通过
$ actionlint（新/改工作流）                          # 0
```

首个 `studio-v*` 发布与 `tauri build` 三平台产物待用户配置 secrets 后实测（见 blockers.md）。

## 波次 13 改了什么（Electron 退役执行：P1；P4/P5 骨架并行）

用户裁决「全部退役 + 去旧账号 + 更新转本仓库 + 零安装网页」。本波次执行 P1（删源与门禁跟随），
并行产出 P4（Worker 骨架）与 P5（独立注册表仓库骨架）。Go 内核逻辑零改动；Go 工具（repolint /
wire-parity / route-parity）随路径迁移做了必要改动（D17）。

- **删除**：`desktop/`（2423 文件）、`studio.yml`/`release-studio.yml`/`deploy-web-studio.yml`/
  `studio-selftest-windows.yml`/`studio-certum-signing-smoke.yml`、`.signpath/`、
  `cmd/signpath-contract/`；`scripts/` 四个 Electron-only 测试删除、六个脚本裁剪。
- **迁移**：`cmd/studio-manifest`（含 `arch.json` 与 gate 测试）；`internal/frontend/serve/testdata/
  trace_readings.json`。
- **门禁**：wire-parity 镜像常量 → `desktop-tauri/src/port`（表测试绿）；tag parity → Vue
  `state/session.ts`；route-parity → `desktop-tauri/vite.config.ts`；repolint `stylesDir`/size
  豁免/layer host 更新；baseline 受控 `-update -allow-widen`（+141/−78）。
- **配置**：`ci.yml` 去 desktop 面；dependabot/labeler/CODEOWNERS/.gitattributes/.gitignore 清理；
  `make studio`→Tauri；`bundle.active=true`（本机 MSI 4.44 MiB + NSIS 2.95 MiB）。
- **P4 骨架**：`workers/remote/`（配对/换票路由、Durable Object 中继、README）＋
  `deploy-remote.yml`（已含构建 `desktop-tauri/dist`，actionlint 通过）。
  已知后续：浏览器 WS 不能带 Authorization（改走子协议/查询参数）。
- **P5 骨架**：独立仓库 `reasonix-registry`（本地已 `git init` + 一次提交，未推远端）：四类内容
  目录、schema、`build-index.mjs`/`validate-schemas.mjs`、validate/publish 工作流、CODEOWNERS、
  PR 模板、两个示例；`registry.pub` 与项目内嵌 minisign 公钥一致（无私钥入库）。

**波次 13 验证（主 Agent 重跑）**：

```
$ go build ./... && go vet ./...                       # 0
$ golangci-lint v2.12.2（GOTOOLCHAIN=go1.26.7）run ./...  # 0 issues
$ go run ./tools/repolint                              # clean (3556 baselined)
$ mingw32-make check                                   # 0
$ go test ./... -p 4 -timeout 20m                      # 211 ok；5 个路径遗漏失败已重指修复
$ (前端) vue-tsc / vitest / vite build                  # 0 / 311 文件全绿 / 构建成功
```

- 首次全量在并发负载下两处超时/环境失败（boot 10 分钟超时、proc job object），单独复跑 PASS，判负载假阳性。
- 温和并发复跑：211 包 ok；5 个失败均为 Go 对等测试仍读已删 `desktop/` 路径（P1 遗漏）。
- 15 个文件重指 `desktop-tauri` 后，10 个受影响包 `-count=1` 全绿（serve 733s、control 198s）。
- sandbox/builtin 3 个失败在 `pre-electron-retirement` 标签 worktree 同样复现：环境性预存项，记 blockers.md。

## 波次 12 改了什么（D15 收口）

用户指令：窗口已验证可开、提交、D5、D15、以完成 Vue/TS/Tauri 重构作为 Electron 退役前提
（Rust+Go 共存不变）。Go/Rust 内核零改动。共四路并行研究（只读）＋三路并行实现（新文件）
＋主 Agent 接线/裁决/收口重跑。

- **D5（done）**：删 pinia（无 import；lock −91 行），提交 `d3683628c`。
- **B10c job 查看器（done）**：`Pane` 提供 JobsLive（jobs+endedJobs、cancel、open→openJob），
  `WorkbenchPanel` 对 job Surface 渲染既有 `JobRunView`；工具卡 BackgroundNote 打开按钮可达。
  `jobcancel.ts` 转 reached（debt 82）。
- **B10d 回到分组（done）**：`Transcript` 真正消费 `focus`——隐藏持有、跨压实折叠/activity
  切片/关闭 details 逐层展开后居中滚动、按 n 一次消费；`Pane` 持有 landAt 并转 `onShowCall`。
  `Transcript.focus.test.ts` 4 例。
- **B11 Plan + conversation-files（done）**：新增 `ui/Plan.vue`（Metrics ORDER 增 `plan`，
  Pane 传 `s.plan`/`blocked`）；新增 `ui/useConversationFiles.ts`（一次性 request、授权夹、
  GRANTED 图片、LocalRefs 八成员、右键菜单、platform），Pane 并入 reply 并承载
  WorkbenchFileMenu；`WorkbenchPanel` 接 `fileRequest`（挂载即消费）与 `onActiveFile`，
  读视图 Markdown 拿 `documentRefs`。测：Plan 6 + convfiles 11 + 工作台链接 1。
- **B14 kept versions + move-to（done）**：`SessionMenu` 增 kept/move 项；`Sidebar` spread 集合
  ＋新 `KeptRows.vue`（versions/copies 整段打开、内联确认后 `hub.removeSession`+reload）；
  `useSessionMoveTo.ts`＋`MoveSessionDialog.vue`（remembered 非源项目、note/reason、持有中禁移）。
  动作名按 census：菜单 `session.move-to-start`、提交 `session.move-to`。测 10 + 6。
- **B12 并发开排序（done）**：`useSessionSwitch` 进 `openPane`（opening 早退、begin 取票、
  flights 合并、current 决定 active/landed、end 收尾）；聚焦/切 tab 调 `cancel()`；
  Sidebar 增 `opening`/`openingPending`；`newSession(root)` 保留 root。`sessionswitch.ts`
  转 reached（debt 81）。测 4 + App 契约。
- **终态保留**：B13（停靠列架构）、`sidebaropen.ts`（D13 内联）、`openCards.ts`（另一条
  card-opening 链）。

**波次 12 验证（主 Agent 重跑）**：

```
$ cd desktop-tauri && ./node_modules/.bin/vue-tsc -b --force   # EXIT 0
$ ./node_modules/.bin/vitest run                               # 311 文件 / 2297 测试全绿; EXIT 0
$ 定向（sidebar 家族 + e2e + App.port + orphan + workbench + focus + convfiles +
    Plan + sessionswitch + move-session + background-job）    # 16 文件 / 145 测试绿
$ grep -c '^  \["src/' src/orphan.test.ts                      # 81
```

基线 304/2250 → **311/2297**（+7 文件 / +47 测试）；debt 83 → **81**。Go/Rust 门未触发（零改动）。

## 波次 6 改了什么

补齐 Tauri 壳对 Electron 主进程的能力对等，并落地内嵌原生 agent-browser 子 WebView：

- `commands.rs::NAMES` 从 16 条增到 **31** 条；`build.rs` 的 ACL 清单同步 31 条，
  `bridge.rs` 的精确名单守卫（`the_bridge_names_exactly_the_declared_commands`）绿。
- 新增六个系统能力模块：`capture.rs`（窗口截图）、`graphics.rs`（GPU/软件渲染状态）、
  `shortcut.rs`（全局快捷键）、`procmem.rs`（壳进程内存）、`textmenu.rs`（文本右键菜单）、
  `dropwin.rs`（Windows 原生拖放路径事件）。
- 新增 `browser/` 模块：子 WebView 的创建/定位/显隐/导航/关闭、按 target 隔离的持久化
  存储、导航与弹窗策略、下载拒绝、权限拒绝、登录与证书询问、元素挑选。内核 origin 与
  壳桥接对子 WebView 不可达（凭据 cookie 只设在主 webview）。
- `bridge.rs` 注入 `window.reasonixHost` 的方法从 16 增到 **33**：`ShellBridge` 声明 34 个，
  唯一未注入的是 `pathForFile` —— 刻意移除，由原生拖放路径事件 `onDropPaths` 取代。
  浏览器登录/加载状态经 event 插件（`browser://login`、`browser://load-state`）订阅。
- 能力声明只紧不松：无平台等价物的动词显式回答 unsupported，不伪造成功；
  `graphics` 读不到 GPU 状态时回答 unknown；软件渲染设置真正写进启动参数
  （Windows `--disable-gpu`，macOS/Linux 的 `additional_browser_args` 为 no-op，如实标 partial）。
- 修两处前端测试：`host_drop.test.ts` 的闭包窄化类型错误、`onboarding/firstrun.test.ts`
  的 `shellPicks` 用对象展开丢失原型方法（`onDropPaths`）导致挂载抛错。

**未验证（受 B1 限制）**：原生子 WebView 的隔离、拖放路径合成只在 policy/pick/protocol
的单测边界证明，未在真实 WebView2 窗口里跑过。macOS/Linux 的子 WebView 与拖放未在本机
（Windows）运行，标 partial，不声称已验证。

## 波次 7 改了什么（进行中：孤儿接线）

子 Agent 类型本机不可用（B8），主 Agent 串行把已落地但未接线的生产组件挂到正确的
 owning 层，逐条削减孤儿债务。每接一条：改 owning 层 → 跑 vue-tsc + 邻近测试 +
 orphan 守卫 + 全套 → 从 `KNOWN_ORPHANS` 删该条。

- **Welcome.vue 已接线（债务 114 → 113）**。React 基准里 `Welcome` 是 App 级「已配置
  但初次见面」的开场场景（`App.tsx` 在 `!welcomed && !setup.required` 分支渲染），
  与 Onboarding 的内联 welcome 步骤是两件事。Vue 端此前把两者塌缩成
  `guide = !welcomeSeen`，且该值在 `onMounted` 一次性快照——而 `welcomeSeen()` 是
  Tauri invoke，挂载后才返回，故生产里向导**永远不会反应式弹出**（新机器直接进 app）。
  修复落在 owning 层：抽出纯函数 `ui/firstRunScreen.ts`（可单测的屏幕判定），App.vue
  的 `welcomeSeen`/`setup` 改为 `boolean | undefined`，门控改成反应式 computed：
  未 settles 或 deferred → app；欠 key（`setup.required`）或 forceGuide → guide；
  已配置但未见 welcome → welcome 场景；否则 app。Welcome 的 `onStart` 置 forceGuide
  打开向导，`onSkip` 走 `finishGuide({})` 标记 seen。新增 `firstRunScreen.test.ts`（6 例）。
  验证：vue-tsc 0；firstrun + onboardingReachable + firstRunScreen 18/18；orphan 3/3；
  全套 **287 文件 / 2154 测试**；vite build 0。

- **文档预览簇已接线（债务 113 → 106）**。React 基准的 `DocumentPreview` 按文件类型
  分派查看器；Vue 端只落地了表格（CSV/TSV）与 HTML 两个查看器，PDF/Office 查看器
  **未移植**。新增 `ui/document/DocumentPreview.vue` 作分派器：用 `useDocument` 的
  getter（非 ref，解构会丢反应性）取 info/data/error，按 `registry.previewKind` 分派
  DelimitedPreview / StaticHtmlPreview / 图片，未支持的类型（pdf/office）诚实回退
  「preview not available」，过大文件给 open-external。`WorkbenchPanel.vue` 启用此前
  硬关闭的 document 模式：`hasDocument`/`source` computed 驱动 FileBar 的模式按钮，
  filePath 变化时按 React 的 mode effect 自动选模式，`documentState` 按文件保存视图状态。
  新增 `design/document.css`（React document.css 的已移植子集，无注释；字面量改用
  `--r-sm`/`--fs-ui` token 以过样式守卫）并在 design.css 引入。新增
  `DocumentPreview.test.ts`（4 例）+ WorkbenchPanel.interaction 文档模式邻接测试（1 例）。
  从 `KNOWN_ORPHANS` 删 7 条已到达文件（useDocument/DelimitedPreview/delimited.worker/
  parseDelimited/StaticHtmlPreview/staticDocument/viewState）。
  **遗留（partial）**：officeSheet/officeWord/deliveries/blocks 仍孤儿——PDF/Office
  查看器未移植，记入 blockers，待后续波次补齐或明确移出范围。
  验证：vue-tsc 0；DocumentPreview 4 + useDocument 3 + workbench_panel 16 +
  interaction 5 = 28/28；样式守卫 44 文件 / 154 例；orphan 3/3；
  全套 **288 文件 / 2159 测试**；vite build 0。

- **图表渲染簇已接线（债务 106 → 101）**。Vue 端早已落地全套图表机件
  （spec/plots/model/format/ChartTable、`styles/chart.css`、i18n 键、chart 图标），
  缺的是分派卡 `ChartCard` 与 Transcript 接线。新增 `ui/chart/ChartCard.vue`
  （React `ChartCard.tsx` 的对等移植：describe/axes、按 mark.type 分派 BarPlot/LinePlot/
  PiePlot、Legend、带 ResizeObserver 的横向 Scroller、hatch 图案、figure + ChartTable）。
  **owning 层修复**： live `Transcript.vue`（Pane 挂载）用本地 `grouped` 把所有 tool
  归入 activity（`inActivity=true`），而 React 基准 `turnrows.transcriptRows` 的规则是
  「带图表的 tool 不算 work」（`!chartOfTool(tool)`），故图表卡以 plain 行渲染。Vue 的
  `isWork` 缺这条排除，导致图表永不显示。按基准把 `isWork` 改为
  `(tool && !chartOfTool(tool)) || reads || worked || guardian`，图表 tool 遂以
  plain 行进入 `card(item,false)`，tool 分支用 `chartOfTool` 判定后以 `Boundary`
  （fallback=ToolCardSafe，retryKey=args）包 ChartCard。注：规范 `transcriptRows`
  已在 `turnrows.ts` 忠实移植，但只被孤儿 blocks/deliveries 路径与测试消费，live 路径
  用的是简化 `grouped`；本次只在 `grouped` 补图表排除，不动整套分组架构（避免无关重构）。
  新增 `ChartCard.test.ts`（3 例）+ `Transcript.chart.test.ts`（3 例：settled 图表卡 /
  普通 tool 仍折叠 / 未 settled 图表 tool 走 tool 卡）。从 `KNOWN_ORPHANS` 删 5 条
  （ChartTable/format/model/plots/spec）；`fixtures.ts` 仅测试导入，保留孤儿。
  验证：vue-tsc 0；chart 邻接（model 11 + spec 31 + turnrows 23 + ChartCard 3 +
  Transcript.chart 3）71/71；orphan 3/3；全套 **290 文件 / 2165 测试**；vite build 0
  （3246 模块）。

- **扩展页运行时重载已接线（债务 101 → 100）**。`useRuntimeReload`（`ui/RuntimeReload.ts`）
  早已落地（offer/run/ok/bad 四态、kernel 拒绝原因经 `reason(e)` 透传、4s 后回落），
  但 live 扩展页 `settings/sections/ext.vue` 未消费它。按 React `Settings.tsx` 的
  ext-foot 范式接线：在 external-services 卡 skills 组之后加 `.ext-foot`（refresh 图标 +
  `settings.changes_apply_from_the_next_turn` + 重载按钮）与 `reload.note`。**owning 层修复**：
  hook 返回的 `action`/`note` 是 `ComputedRef<VNode>`，模板里 `reload.action` 是嵌套 ref，
  Vue 只自动解包顶层 ref，故 `Content` 收到的是 ref 对象而非 VNode、按钮永不渲染。改为在
  setup 顶层解构 `const { action: reloadAction, note: reloadNote } = useRuntimeReload(...)`，
  模板遂正确解包。新增 `ext.reload.test.ts`（3 例：footer 提供重载动作 / 成功透传
  `app.live_the_next_turn_runs` 并触发 onChanged / 拒绝透传 kernel 原因）。从 `KNOWN_ORPHANS`
  删 1 条（RuntimeReload.ts）。验证：vue-tsc 0；ext.reload 3/3；orphan 3/3；
  全套 **291 文件 / 2168 测试**；vite build 0。

- **设置可发现性 parity 守卫已移植（不减债务，增验证）**。React
  `settings-findability.test.ts` 用 `INTENTS`（自然语言查询 → 期望 anchor）断言 live
  搜索 `searchSettings` 把目标设置放进前三结果、catalogue 含全部 anchor、无关词无命中、
  多词收窄。忠实移植为 `ui/settings-findability.test.ts`（4 例），对 Vue live 搜索全绿——
  证明 Vue 设置搜索与 React 可发现性契约行为一致。`settings-intents.ts` 是测试 oracle
  （React 侧也仅被该测试导入），按 orphan 守卫口径测试导入不计可达，故与 `chart/fixtures.ts`
  一样保留为测试_only 孤儿，不接线、不减债务。全套增至 **292 文件 / 2172 测试**。

- **远程感知 boot 根已接线（债务 100 → 98）**。React `main.tsx` 用 `RemoteAwareApp`
  包 `App`，叠加两类通知：托管页是旧 build 时的 stale-page 提示（info，刷新），以及
  远程连接结束时的通知（reauth → 重新登录；带 failure → `failureView` 的具体标题/正文/
  动作/waitS/retryWhenOnline；否则通用重连）；boot 失败若是 `RemoteLinkError.failure`
  则渲染 `RemoteNotice` 而非通用 errbar。Vue 机件早已齐全（`cloud_remote`/`cloud_failure`/
  `cloud_link`/`stale_page`/`RemoteNotice.vue`，i18n app.* 与 remote.* 键全在），但 live
  `main.ts` 简化掉了：成功直接 `createApp(App)`、失败一律 errbar。按基准新增
  `ui/RemoteAwareApp.vue`（useStalePage + onRemoteConnectionEnded + failureView +
  signInAgain/openRemoteHome，本地 failureLabel/failureAction 与 React 一致），`main.ts`
  成功改挂 `RemoteAwareApp { hub, remote: remoteDeviceId() }`、失败分支判 `RemoteLinkError`
  后 `createApp(RemoteNotice)`（否则保留原 escape(errbar)，无新增 XSS 面）。新增
  `RemoteAwareApp.test.ts`（5 例：本地无通知 / stale info 提示 / reauth 重新登录并触发
  signInAgain / 具体 failure 透传 kernel 标题 / ended 取代 stale 不叠加；App 以 stub 隔离）。
  从 `KNOWN_ORPHANS` 删 2 条（RemoteNotice.vue、stale_page.ts）。验证：vue-tsc 0；
  RemoteAwareApp 5/5；orphan 3/3；全套 **293 文件 / 2177 测试**；vite build 0。

- **窗口内链接路由已接线（债务 98 → 97）**。`useLinkRouting`（`ui/links.ts`）早已落地：
  拦截窗口内 `a[href]` 的 http(s) 点击，默认开进本窗口所绘浏览器（`openPage` + reveal，
  模型可见可驱动），修饰键或壳不绘视图时走机器浏览器（`openExternal`），页失败且无自身
  tab 时回落 external。React `App.tsx:409` 调 `useLinkRouting(activePort, () => setBrowser(true), fail)`，
  Vue `App.vue` 简化掉了。按基准在 `App.vue` setup 接 `useLinkRouting(activePort, () => showView("browser"), fail)`
  （activePort/fail/showView 均早已存在）。新增 `links.test.ts`（5 例：http 开进窗口浏览器并
  reveal / 修饰键走 external / 壳无视图走 external / 非 http 不拦截 / 页失败无 tab 回落 external；
  mock host.drawsBrowserViews 与 openPage 隔离）。从 `KNOWN_ORPHANS` 删 1 条（links.ts；
  browser_open 早已可达）。验证：vue-tsc 0；links 5/5；orphan 3/3；全套 **293 文件 / 2176 测试**
  （两次连跑稳定，全绿）；vite build 0。

- **活动 pane 跨重载存活已接线（债务 97 → 96）**。`ui/activepane.ts`（`savedActivePane` /
  `rememberActivePane`，sessionStorage `rx-active-pane`）早已落地。React `App.tsx:110`
  `useState(savedActivePane)`、`:160` `useEffect(rememberActivePane(active),[active])`；
  Vue `App.vue` 曾 `active = ref("")` 丢掉存活。按基准接：`App.vue:70`
  `const active = ref(savedActivePane())`、`:485` `watch(active, (id) => { report.value = NO_REPORT; rememberActivePane(id); })`。
  从 `KNOWN_ORPHANS` 删 1 条（activepane.ts）。

- **等待提示按基准移植 Await，重试线已接线（债务 96 → 95）**。`state/retry_line.ts`
  （`retryPhase`：delayMs 内是 backoff，之后是 attempt 自身等待）早已落地但无消费点。
  Vue `Transcript.vue:302` 曾画静态 `div.await` 只读 `waiting.ttftSince`、忽略 `waiting.retry`，
  连既有的三点动画 CSS（`app.css:930-936`，与 React 同）都没用上。React `Transcript.tsx:513`
  `<Await since retry>`、`Await.tsx` 按 cause/scope 报名 + 走秒时钟。新增 `ui/Await.vue`
  忠实移植：`lead(retry)` 五 cause 回落 scope、`phase(retry,elapsed)` backoff/replay/timeout/
  default 四变体（max=0 读作 ∞）、`start=retry?.since??since` 走秒（`useShown` 门控 100ms
  interval，无 PaneShown provider 故默认 true）、三点 `<i/>` + `.t`、`.await[data-retry]`。
  `Transcript.vue` import Await 并以 `h(Await, { since, retry })` 替换静态 div（保留 role=status
  这一既有 Vue 无障碍属性）。新增 `Await.test.ts`（6 例：plain 走秒 + 三点 / 挂载期间走秒 /
  stream replay backoff 报 cause / headers timeout 带 limit / max=0 读 ∞ / 无 cause 回落 scope；
  fake timers 驱动时钟）。从 `KNOWN_ORPHANS` 删 1 条（retry_line.ts）。
  验证（activepane + Await 两簇合并跑）：vue-tsc 0；Await 6/6；orphan 3/3（债务 **95**）；
  全套 **294 文件 / 2182 测试**全绿；vite build 0。

- **侧栏反馈未读计数已接线（债务 95 → 94）**。`ui/useFeedbackUnread.ts`（轮询保未读诚实：
  仅有未结报告者被轮询、背景读失败静默、隐藏窗口只为要求通知者轮询、machine 变更重启）早已
  落地但无消费点；`App.vue` 曾把 `feedbackUnread: 0` 写死、`onFeedback` 恒开 "send"。React
  `App.tsx:285` `machineRt = runtimes.find(rt => !rt.host)`、`:294`
  `useFeedbackUnread(!FEEDBACK_RETIRED && machineRt ? machinePort : null, machineRt?.id ?? "", feedbackUnread, setFeedbackUnread, feedback !== null)`、
  `:595` `feedbackEntryTab(feedbackUnread)`、`:755` Feedback `onUnread/unread`。按基准接：
  `App.vue` 新增 `machineRt = computed(() => runtimes.value.find(rt => !rt.host) ?? null)` 与
  `feedbackUnread = ref(0)`，调 `useFeedbackUnread`（五个 MaybeRefOrGetter 实参，port/machine/
  panelOpen 用 computed 包），`onFeedback` 改 `feedbackEntryTab(feedbackUnread.value)`，Nav
  `feedbackUnread: feedbackUnread.value`，Feedback 补 `onUnread/unread`（Feedback.vue 早已声明
  这两个 prop 并画 `.fbk-tab-count`，只是没人传）。`feedbackentry.vue` 经 Nav 早已可达，非孤儿。
  从 `KNOWN_ORPHANS` 删 1 条（useFeedbackUnread.ts）。`App.port.test.ts` 加 1 例静态契约
  （App 编译脚本含 useFeedbackUnread 调用、feedbackEntryTab、machineRt 选择、Nav/Feedback 未读
  传递），沿用该文件既有 toContain 口径；hook 轮询行为由既有 `useFeedbackUnread.interaction.test.ts`
  21 例覆盖。验证：vue-tsc 0；App.port 17/17 + interaction 21/21 + orphan 3/3（债务 **94**）；
  全套 **294 文件 / 2183 测试**全绿；vite build 0。

- **剩余 94 孤儿全量分类（第二轮 Explore 审计 + 主 Agent 逐条复核）**。派 Explore 只读
  审计 94 条，报 6 条 CLEAN-WIRE；主 Agent 按「不以子 Agent 自我报告为依据」逐条实测复核，
  **6 条全是误报**，已改判并留痕：
  - `AuthorLine.vue` → **REDUNDANT-INLINE**：`versions.vue:59-62` 早已内联同一 `.comm-author`
    （同 i18n key、同 `data-action="community.author"`、同 authorUrl），按 D13 保留孤儿不強换。
  - `useTabNames.ts` → **SUBSYSTEM-BLOCKED（新增 B13）**：改名 browser-only（React 双击/F2 均
    `kind==="browser"` 门控），而 Vue `WorkbenchPanel.vue:219-223` 的 surfaces 只产
    changes/file/terminal，**无 browser 面**（React :314-319 有），接上是死代码。
  - `sessionExport.ts`/`sessionExportMarkdown.ts`/`sessionInfo.ts`/`useSessionMove.ts` →
    **SUBSYSTEM-BLOCKED（新增 B14）**：React 会话菜单（`Workspaces.tsx:640-660`）是整套子系统
    （auto-name/pause/close/branch/copy-info/export/kept-versions/move/move-to），Vue
    `SessionMenu.vue` 只 rename/archive/delete；`useSessionMoveTo` Vue 无；接线跨
    SessionMenu+Sidebar+WorkspaceGroup+App.vue（主 Agent 所有），属 B10-B12 同类半措施风险。
  复核后 94 条的真实分布：**CLEAN-WIRE 0 / REDUNDANT-INLINE 34 / TEST-FIXTURE 19 /
  SUBSYSTEM-BLOCKED 41**。即剩余孤儿无一可简单接线消债：34 是功能已内联的冗余（D13，非缺口），
  19 是测试夹具（testkit/fixtures/intent 表等，test-only 消费按守卫规则恒孤儿），
  41 依赖未移植子系统（B9/B10/B11/B12/B13/B14 及 transcriptRows/MeterRail/RunAnalysis 等）。
  本轮三簇（activepane/Await/useFeedbackUnread）是最后的干净接线，债务 97 → 94 后进入
  「冗余 + 夹具 + 子系统」平台期。orphan 守卫复跑 3/3，债务 **94** 不变（仅改注释，可达性未变）。

## 工作在哪

主干：`C:\Users\10129\Desktop\Projects\DeepSeek-Reasonix-rs`（裁决见 `decisions.md` D1）。

| 代号 | 路径 | 性质 |
|---|---|---|
| 原项目 | `C:\Users\10129\Desktop\Projects\DeepSeek-Reasonix` | 只读基准，Electron + React + Go |
| A | `C:\Users\10129\Downloads\DeepSeek-Reasonix-rs` | 同一仓库的 Gitee 快照，主干工作的更早状态（crates 骨架已被 B 上游取代，contract golden 切片保留） |
| B（本地，陈旧） | `C:\Users\10129\Downloads\DeepSeek-Reasonix-studio-RS` | M0 `87bfa14`，crates 348 行；**不吸收**（被上游最新版取代） |
| B（上游最新，已采纳） | `C:\Users\10129\Downloads\reasonix-studio-rs-main` | Gitee main `9b3d7ba`，M2；D14 合并来源，clone 留作记录 |

## 分层状态

| 层 | 状态 | 可核对的证据 |
|---|---|---|
| Go 内核（`internal/`、`cmd/`） | 完成，未被改写 | `go build ./...` 退出 0；`go vet ./...` 零行输出；`git status` 对这两处零改动 |
| Tauri 壳（`src-tauri`） | 可用 | `cargo check`/`clippy --all-targets -D warnings`/`fmt --check` 退出 0；`cargo test` 90 绿；`NAMES` 31 条命令 |
| Vue 前端（`desktop-tauri/src`） | **partial（剩余为终态裁决）** | typecheck 0；311 文件 / 2297 测试全绿；build 0；327 `.vue` / 799 `.ts` |
| Electron 界面（`desktop/`） | 完成，基准冻结 | 未改动；Tauri 对等后才退役（D4） |
| Rust 内核（`crates/`＋`apps/`＋`xtask/`） | **partial（M2，进行中）** | D14 采纳上游 B；`cargo test` 572 绿、fmt/clippy/xtask parity(563 行)/deps 全 0；账本 69 done／493 todo；host 仍 stub，未 cutover（B15） |

Vue 前端为什么是 partial：`orphan.test.ts` 用 81 条具名记录量出来的（波次 5 是 129，
波次 7 削 35→94；波次 9–11 续削至 83；波次 12 再削 2（jobcancel、sessionswitch）至 81）。
这 81 个文件类型检查通过、单测通过，但从 `main.ts` / `testsetup.ts` 出发没有任何 import
路径能到达它们；其中没有未落地的用户可见子系统——分布是冗余内联（D13）、测试夹具与终态裁决
（B13 架构分歧、`sidebaropen.ts` 内联、`openCards.ts` 另一条链）。详见 blockers/decisions D16。

波次 5 的第二处缺口（`ShellBridge` 33 方法对 `bridge.rs` 16）已收口：`ShellBridge` 现声明
34 个方法，`bridge.rs` 注入 33 个，唯一缺的 `pathForFile` 是刻意移除（见上）。残差见
`migration-plan.md` 的 IPC 对照。

## 已验证基线（波次 6 实跑）

```
$ cd desktop-tauri && ./node_modules/.bin/vue-tsc -b --force; echo EXIT=$?
EXIT=0

$ cd desktop-tauri && ./node_modules/.bin/vitest run 2>&1 | tail -4
 Test Files  286 passed (286)
      Tests  2148 passed (2148)

$ cd desktop-tauri && ./node_modules/.bin/vite build 2>&1 | grep -E "modules transformed|built in"
✓ 3225 modules transformed.
✓ built in 19.17s

$ go build ./... ; echo GO_BUILD=$?
GO_BUILD=0

$ go vet ./...        # 无输出，日志 0 行
GO_VET_EXIT=0

$ cd desktop-tauri/src-tauri && cargo fmt --all -- --check; echo FMT=$?
FMT=0

$ cd desktop-tauri/src-tauri && cargo clippy --all-targets -- -D warnings 2>&1 | tail -1
    Finished `dev` profile [unoptimized + debuginfo] target(s)

$ cd desktop-tauri/src-tauri && cargo test 2>&1 | tail -1
test result: ok. 90 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out

$ sed -n '/pub const NAMES/,/];/p' src-tauri/src/commands.rs | grep -oE '"[a-z_]+"' | wc -l
31

$ sed -n '/^interface ShellBridge/,/^}/p' src/port/host.ts | grep -cE '^  [a-zA-Z]+\??\('
34

$ find desktop-tauri/src -name "*.vue" | wc -l
317
$ find desktop-tauri/src -name "*.ts" | wc -l
770
$ find desktop-tauri/src -name "*.test.ts" | wc -l
286
$ grep -c '^  \["src/' desktop-tauri/src/orphan.test.ts
114
```

孤儿债务条数口径：`grep -c '^  \["src/' src/orphan.test.ts` = **81**（波次 12 实测；波次 11
记录 83，本波次 jobcancel.ts/sessionswitch.ts 转 reached；波次 6 快照 114，波次 5 是 129）。
`bridge.rs` 注入方法数用逐名 grep 核对：34 个 `ShellBridge` 方法里 33 个命中，
`pathForFile` 命中 0（刻意移除）。

## A 与主干的实测 diff

口径写在命令里，可复现。

```
$ A="C:/Users/10129/Downloads/DeepSeek-Reasonix-rs/DeepSeek-Reasonix-rs"
$ diff -rq --exclude=node_modules --exclude=.git --exclude=target \
       --exclude=dist --exclude=vendor --exclude=.reasonix \
       --exclude=gen --exclude=public "$A" .
$ # 目录条目展开为文件后计数
仅 A 有   : 5 个文件
仅主干有  : 206 条目录条目 → 展开 1262 个文件
内容不同  : 71 个文件
```

展开后的分布：1208 个在 `desktop-tauri/src`，30 个在 `desktop-tauri/src-tauri`。
其余是 `docs/migration-*.md` 六份、两份 `.exe`、若干构建日志与 `tmp-*.mjs`。

仅 A 有的 5 个文件全列在下表。

| A 独有文件 | 处置 |
|---|---|
| `desktop/frontend-next/src/ui/SidebarNav.tsx` | 只作 Vue Sidebar 的参考，不复制 |
| `desktop-tauri/src/port/wire.parity.test.ts` | 测已删的 `port/kernel.ts`，见 B4 |
| `desktop-tauri/src/App.vue` | 已被 `ui/App.vue` 取代 |
| `desktop-tauri/src/stores/kernel.ts` | 已被 `state/*` 取代 |
| `.github/workflows/v1-maintenance-schedule.yml` | v1 线已退役，无对应物 |

口径差异要标出。任务书写「仅主干 1197 个」，本波次在上面那条命令下是 1262。

5 与 71 两版完全一致。主干侧的差异只来自排除集：

| 排除集 | 仅主干文件数 |
|---|---|
| 只排 `node_modules` / `.git` / `target` | 2023 |
| 再排 `dist`，展开目录 | 1285 |
| 再排 `vendor` / `.reasonix` / `gen` / `public` | 1262 |
| 只数源码扩展名，去点文件与日志 | 1244 |

没有一个口径能精确复现 1197，所以这里不硬凑。本文给出命令与展开规则，读者可自行复算。

## B 的规模

**本地 B（陈旧，不吸收）**：25 个 `.rs`、1585 行；一条提交 `87bfa14 M0`。ledger 19 行：
11 done / 1 wip / 6 todo。

**上游最新 B（已采纳，D14）**：Gitee main `9b3d7ba`，**M2**。87 个 `.rs`、33 751 行。
账本 563 行＝**69 done / 1 wip / 493 todo**。按 crate：state 22 747 行、contract 4 807、
core 4 641、cli 182、assembly 34，其余（agent/tools/provider/serve/safety/platform/ext/
acp/tui/testkit/session/config）8–11 行 skeleton；apps 90 行。M1 契约清单已全部冻结
（`docs/contracts/` 14 份），M2 实现 session-format（SESS）与 provider-message（PMSG）。

## 组件普查（波次 5 实测）

| 口径 | 数值 |
|---|---|
| React `frontend-next/src/ui` 下 `.tsx` | 1141（非测试 438） |
| Vue `desktop-tauri/src` 下 `.vue` | 317 |
| Vue `desktop-tauri/src` 下 `.ts` | 768（`.test.ts` 284） |
| React 侧非测试模块名 | 675 |
| Vue 侧非测试模块名 | 755 |
| React 有、Vue 无同名文件 | 186 |
| Vue 有、React 无同名文件 | 266 |

「186」不能当缺口数。设置中心把 20 个 section 合成一个 `Settings.vue`，加
`settings/sections/` 20 个与 `settings/parts/` 48 个 part，名字自然不一一对应。

反过来 Vue 侧那 266 个里也有大量 `use*` 组合式函数。诚实的缺口量是 114 条债务（波次 6）。

## 已落地但未接线（114 条）

统计口径：从 `orphan.test.ts` 抽出 `KNOWN_ORPHANS` 的路径，按前缀分组（波次 6 实测）。

| 类别 | 条数 | 说明 |
|---|---|---|
| `src/ui/**` | 101 | `.vue` 组件 + `.ts` 工具/组合式函数（波次 5 是 115） |
| `src/state/**` | 5 | 执行图、panes 读取、恢复、重试行、rewind 提示（波次 5 是 6） |
| `src/actions*` | 6 | 动作普查表，读者是 vitest 守卫而非窗口 |
| `src/e2e/**` | 2 | `graph_observer.ts`、`shell_shim.ts`，包外驱动 |
| 合计 | 114 | 与 `grep -c '^  \["src/'` 一致 |

按功能看，仍未挂上界面的典型项：

- `ui/panels/Files.vue` —— `Metrics.vue` 挂了 8 个兄弟面板，唯独没挂它。
- `ui/remote/RemoteProjectMenu.vue`、`RemoteSessionRow.vue` —— 远端项目菜单与会话行。
- `ui/chart/*` —— 图表渲染器齐了，但没有卡片把 spec 交给它。
- `ui/document/*` —— 工作台还没按文件类型选查看器。
- `ui/surface/*` —— Pane 的 slot 视图还没按名字挂载它们。
- `ui/kit/Button.vue` —— 共享原语，尚无调用点选它。
- `ui/cards/SubagentGroup.vue`、`ui/cards/openCards.ts`。
- 单点组件与组合式函数一批：`Welcome`、`WorkbenchLauncher`、`ChangedFilesPill`、
  `NewAnswerLine`、`WorkFold`、`Sky`、`TariffChip`、`useNewConversation`、`usefind`、
  `userewind`、`useAgentTabs` 等。

## 刻意保持的非目标

| 项 | 为什么不是缺口 |
|---|---|
| `settingPages.PAGES` 为空 | 原基准里就是桩，两边都空 |
| `FEEDBACK_RETIRED` 关闭反馈面 | 原设计如此，两边一致 |
| `Section` 20 个 / section 文件 20 个 | 实测两个集合相同，无遗漏 |

## 下一动作

1. **Rust 内核 parity 推进（D14 采纳后）**：按上游 B 的 M3–M6 收敛账本 493 条 todo
   （serve/provider/tools/agent/safety），golden fixtures 对等，host 实现 HOST 行。
   每 slice 以 `cargo xtask parity --check` 与测试为证据。**cutover 阻塞见 B15**：
   账本未收敛、host 仍 stub 前，Go host 保持 live，不切换。
2. **补跑供应链门**：安装 `cargo-deny`（＋nextest/machete）后跑 `cargo deny check` 等；
   本机未装，当前 skipped（B15）。`deny.toml` 策略已随 B 采纳。
3. **Vue 前端**：D15 已收口（波次 12）。剩余 81 孤儿＝冗余内联（D13）＋测试夹具＋
   **终态裁决**（B13 架构分歧；`sidebaropen.ts` 内联；`openCards.ts` 另一条链）。无未落地
   的用户可见子系统；Electron 退役的用户面前提已满足（窗口由用户验证可开），退役本身的
   删除动作待用户确认后执行（见 `migration-plan.md` D4 顺序）。
4. **真实窗口验证**：见 `blockers.md` B1。拿到一次成功启动前，只在 WebView2 里才看得出
   差异的东西都是未验证（原生子 WebView 隔离、拖放合成、登录/证书/挑选）。macOS/Linux 未在本机运行。
5. **Electron 退役**：顺序已写入 `migration-plan.md`（7 项）。波次 12 已完成第 3、4 项
   （oracle 对照冻结为 `parity-contract.json`/`oracle-named.json`；Makefile 默认切 Tauri，
   Electron 保留冻结目标）。**待用户决策**：Tauri 打包（`bundle.active=false`）、Tauri 发布
   工作流、Electron 三条 CI/部署工作流的删除与网页版去留；均确认后再执行删除 `desktop/`
   （2423 文件）。在此之前 Electron 只读冻结，Tauri 是唯一开发目标。

## 波次 15 改了什么（P4 零安装网页访问）

Worker 网关语义从骨架补齐：配对/换票/中继三角色的完整契约见 `workers/remote/README.md`；
Go `remotecloud` 在 account.Client 之外新增 Registrar/NewRelay，serve 提供
`/remote/relay*`；Vue 恢复加密链路并新增 hosted 配对页与设置卡（D20）。

**波次 15 验证**：

```
$ tsc --noEmit（workers/remote）               # 0
$ go build ./... && go vet ./...                # 0
$ golangci-lint v2.12.2（GOTOOLCHAIN=go1.26.7）  # 0 issues
$ go test serve/remotecloud/cmd -count=1         # 全绿（serve 748s）
$ vue-tsc -b --force                            # 0
$ vitest run                                    # 303 文件全绿
$ vite build / repolint / actionlint            # 全绿
```

部署前置：Cloudflare KV namespace id 与 API secrets 仍需用户配置；端到端实机尚未执行。

## 波次 16 改了什么（P5 签名注册表接入）

`reasonix-registry` 实仓上线（签名发布已验证）。内核以签名索引替代旧账号市场，
详见 D24。执行方式：执行代理产出主体，主 Agent 接手收尾（索引/内容源分离、跳转策略、
process-input/孤儿/现代 lint 修正）并做线上实拉。

**波次 16 验证**：

```
$ go build ./... && go vet ./...                    # 0
$ golangci-lint v2.12.2（GOTOOLCHAIN=go1.26.7）      # 0 issues
$ go test ./internal/ext/registry/ ./internal/ext/market/   # 绿
$ go test ./internal/frontend/serve/ -run "Market|Registry" # 绿
$ go run ./tools/repolint -changed                  # clean
$ 线上实拉：List 4 条；Resolve+Fetch release-notes sha256 通过
```

## 波次 17 改了什么（无域名跨网络直连）

设备自身的公网 IPv6 + 自签 HTTPS 直连门（D25）。中继与局域网保留为兜底，不是主路径。

**波次 17 验证**：

```
$ go build ./... && go vet ./...                    # 0
$ go test serve -run "Public|PairLimiter|Share|Refusal|Parity|Device"  # 绿
$ golangci-lint v2.12.2（GOTOOLCHAIN=go1.26.7）      # 0 issues
$ go run ./tools/repolint -changed                  # clean
$ vue-tsc -b --force                                # 0
$ vitest run                                        # 304 文件全绿
```

入站端口放行、手机 IPv6 与首次证书警告需用户侧确认。
