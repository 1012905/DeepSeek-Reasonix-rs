---
owner: "@SivanCola"
backup: "@esengine"
status: active
reviewed: 2026-11-17
---

# 阻塞与跳过的工作

每条都写明：试了什么、被什么挡住、解锁条件是什么。凡 `migration-plan.md` 里
不是 `done` 的交付物，都在这里带着原因。

最后更新：2026-11-18（波次 8：采纳上游 Rust 内核，D14）。

## Open

### B1 —— 本机无法把 Tauri 窗口跑起来

- **试了什么**：直接启动壳，让 Vue 界面在真实 WebView2 里被驱动。
- **被什么挡住**：release 二进制启动后立即退出，退出码 1，stdout/stderr 全空。
  debug 二进制能让 host 起来并打出 `host: handshake from http://127.0.0.1:6442`，
  但随后报 `the kernel exited with 1 before saying anything; retrying once`，
  重试后 `startup failed`。
- **直接原因**：host 日志显示内核跑完 `runtime` 阶段后在 `cleanup stale subagents`
  失败，`acquire parent session lease "...deepseek-v4.1-flash": Access is denied.`。
  另有 `secure lock directory: chmod ...\locks\config-edits-a27e2bb0209f0d07:
  Access is denied.` 与多份 `.jsonl.lock: Access is denied.`。
- **判断**：这是当前会话进程的锁文件权限问题，不是壳或界面的缺陷。
- **后果**：界面验证只能跑在 `vite dev` / `vitest`（jsdom）。只在真实 WebView2 里
  才表现出的差异都是**未验证**。波次 1–5 的「界面已接线」结论来自源码与单测，
  不来自一次成功的窗口启动。波次 6 新增的原生子 WebView 隔离、拖放路径合成、
  登录/证书询问、元素挑选同样在此列 —— 它们的策略与答案清洗在
  `policy.rs`/`pick.rs`/`protocol.rs` 单测边界证明，但「在真实窗口里确实如此」未验证。
  macOS/Linux 的子 WebView 与拖放未在本机（Windows）运行，标 partial。
- **解锁条件**：清掉 `C:\Users\10129\.reasonix\locks\` 与 `...\sessions\*.lock`
  上的占用。多半是同机另几个 `Reasonix Studio.exe` / `reasonix-studio-host.exe`
  持有 —— 实测 `tasklist` 里同时有 4 个前者与 1 个后者。或换干净主机。
- **状态**：跳过，2026-11-16 记录。

### B2 —— MockPort 没有历史分页（裁决：不播种，恢复已在 state 层实测）

- **试了什么**：在浏览器里重开/恢复一段对话；评估给 `port/mock.ts` 的 `history()` 播种。
- **原记录**：`history()` 直接 `return [];`，浏览器会话里没有可恢复的东西。
- **复核纠正（2026-11-18）**：「恢复未实测」不成立——
  - `state/restore.test.ts` 用**连贯的非空 HistoryMessage fixture**（合成 port，非 MockPort）
    实测 `restoreSession`＋`fromHistory`＋reducer：内核任务列表权威、complete_step 推进、
    被拒 todo_write、离线读不 blank、模型归属来自消息而非当前设置、refreshTodos。
  - `state/history_pages.test.ts`、`ui/useHistoryPages.test.ts`／`.http.test.ts` 实测分页投影。
  - `ui/App.port.test.ts:77` 断言 Pane 持有 reducer/history/submit/draft/decision owners（接线）。
- **为何不播种 mock**：`history()=[]` 是**有意**的 fixture 语义（「absent must render as absent」，
  见 mock.ts todos 注释），且 **21 个 UI 测试文件**以 `new MockHub/MockPort` 挂载 fresh-session，
  全局播种会改动这套语义、危及邻域（「破坏邻域的修复是失败的修复」），而新增覆盖已被上述
 合成 fixture 测试提供。属「与迁移合并无关的重构」，按指令记录并跳过。
- **真正的解锁条件**：真实内核会话录像／真实 Tauri 窗口重开——那是 **B1**，与 mock 无关；
  给 mock 播种并不能解锁浏览器实测。

### B3 —— Go→Rust 内核重写的吸收（已解决，D14，2026-11-18）

- **原记录**：本地 B（`87bfa14`，M0，crates 348 行）把 Go 内核重写成 Rust，
  D3 据此判「不在范围内、不合入」，只拟归档其 ADR/ledger。
- **裁决变更**：用户指令采纳 B 的**上游最新版**（Gitee `rikkarikka/reasonix-studio-rs`
  main，`9b3d7ba`，M2，87 个 `.rs`／33 751 行，账本 69 done／1 wip／493 todo）。
  这与主干自身 `RUST_MIGRATION.md` 的既定方向一致；D3「避免两个内核」的张力实为
  A 谱系与 B 谱系之争，由 D14 择 B 上游解决。
- **已做**：按模块合并进 trunk `crates/`＋`apps/`＋`xtask/`，保留 A `contract` 的
  eventwire/pricing golden 切片；Go/壳/Vue 保持 live。验证见 D14 与 migration-state。
- **剩余**：内核未对等、host 仍 stub → cutover 阻塞见 **B15**。

### B4 —— 线缆对等测试：kind 门已恢复，type-witness 对等待契约 reconciliation

- **原记录有误**：旧描述称测试针对 `port/kernel.ts`／`HubPort`。实测 HEAD 的
  `wire.parity.test.ts`（898 行）import 的是 `./wire` 与
  `../../../crates/contract/src/eventwire_manifest.json`，断言 TS wire 类型 ↔ Go
  反射 manifest（67 类型，与 Rust `eventwire_parity.rs` 同一权威），与 kernel.ts/HubPort 无关。
- **试了什么**：从 HEAD 逐字恢复该测试并运行。
- **被什么挡住**：worktree 的 `wire.ts` 已被重构（vs HEAD 500+/598−）：重命名
  `ShellExecution→Execution`、`Attachment→StoredAttachment`、`SurfaceTone→ExtensionViewTone`；
  把 context/lease/surface-rich 类型拆到兄弟文件再 re-export；并删除
  `CompletionReceipt`/`ReceiptChange`/`ReceiptGap`/`ReceiptVerification`/`RecoveryApproval`/
  `VideoShown`/`WorkspaceChanged`/`WorkspacePathChange`/`WorkspaceRevision`/`RateSnapshot`/
  `Valuation`/`Spec`/`SurfaceNode`。manifest 仍声明这些 Go 名。HEAD 测试的 witness 用旧名
  `satisfies wire.X`，故 `vue-tsc` 报 ~20 处 TS2694/TS2353（runtime 136 全绿，因 esbuild 剥类型）。
- **已落地（有界）**：`wire.ts` 增 `KINDS` const 元组，`Kind` 由它派生（单一真相，杜绝漂移）；
  `wire.parity.test.ts` 缩为 **kind-set 对等门**（双向 36↔36，kind 字符串是稳定路由面）。
  实测：该测试绿；`vue-tsc -b --force` exit 0；全量 `vitest` 296 文件 / 2321 测试全绿。
- **解锁条件（剩余 type-witness 对等）**：一份 deliberate 契约 reconciliation —— 把 Vue 名映射到
  Go manifest 名，并据 Vue port 实际消费哪些事件决定被删类型是否须回到镜像。这不是测试恢复，
  属「与迁移合并无关的大规模重构」之外，按指令记录并跳过，留作专门任务。

### B6 —— B 的归档目录冲突（已消解，D14，2026-11-18）

- **原记录**：D3 拟把 B 的 ADR/ledger 归档到 `docs/archive/studio-rs/`，与
  `DOCS_STANDARD.md` C2「Git history is the archive，不加 archive/ 目录」冲突，
  目录也未建，文档指向不存在的路径。
- **消解**：D14 把 B 的 `docs/adr/0001-crate-layout.md`、`docs/parity/LEDGER.md`、
  `docs/contracts/*` **采纳为 trunk live 文档**（不再归档）；B 的 AGENTS/README 存为
  `docs/kernel-rs/` provenance。`docs/archive/studio-rs/` 不再需要，C2 得以遵守，
  文档与文件系统不一致消除。
- **实测**：trunk 现有 `docs/adr/0001-crate-layout.md`、`docs/parity/LEDGER.md`
  （＋BREAKS.md）、`docs/contracts/`（14 份冻结契约）、`docs/kernel-rs/{AGENTS,README}.md`。

### B7 —— 子 Agent 后端 DNS 不稳定

- **试了什么**：波次 2–5 用并行子 Agent 派发任务。
- **被什么挡住**：子 Agent 后端 `chickener.top` 多次 DNS 解析失败，任务在派发或
  执行阶段报错。
- **实测**：失败时段解析不出地址；正常时段 `nslookup chickener.top` 返回
  `104.19.128.100`、`104.17.32.100`（Cloudflare）。
- **真正的危险**：失败与落盘不一致 —— 报错的任务可能已经把文件写进了树。
  所以「任务失败了」不等于「这次改动不存在」。
- **两条纪律**：收口不以子 Agent 自我报告为依据（每个数字都重新跑命令）；
  重派之前先看文件系统，避免两份半成品。
- **解锁条件**：后端 DNS 恢复稳定；或关键路径改由主 Agent 串行执行。
- **状态**：记录在案，非阻塞（可重试），但会放大报告与落盘不一致的风险。

### B8 —— 本波次实测无可用子 Agent 类型（波次 7 降级串行）

- **试了什么**：波次 7 计划按文件所有权并行派发四个接线子 Agent
  （Onboarding/Welcome、remote、workbench/document、cards/chart）。
- **被什么挡住**：`Agent` 工具实测 `general-purpose` 报
  `Agent type 'general-purpose' is disabled via disallowedTools configuration.`；
  `claude` 报 `Unknown agent type: 'claude'. Available types: `（可用列表为空）。
  即当前宿主没有任何可用子 Agent 类型。
- **与 C1 的关系**：C1 曾记录「最多 8 路并行」并作废串行降级。该结论描述的是当时
  那一刻的宿主策略；**本波次实测可用类型为空**，故并行不可用。宿主策略会随时间变化，
  以当波实测为准，不沿用旧结论。
- **后果**：波次 7 降级为主 Agent 串行接线（指令允许「环境不支持子 Agent」时串行）。
  主 Agent 临时承担业务模块接线（角色分工以子 Agent 可用为前提，不可用时由主 Agent 兜底）。
- **纪律**：每个接线串行做、当场跑 vue-tsc + 相关 vitest + orphan 守卫；改动前已快照
  `.migration-backup/wave6-src.tgz`（树多为未跟踪文件，git 无法恢复，快照是回退网）。
- **解锁条件**：宿主重新启用子 Agent 类型（`Available types` 非空）后，可按所有权表并行。
- **状态**：跳过并行，串行继续，2026-11-17 记录。

### B9 —— PDF / Office 文档查看器（已解决，波次 9，2026-11-18）

- **原记录**：Vue 只落地表格/HTML/图片查看器，pdf/office 诚实回退「preview not available」；
  officeSheet/officeWord/deliveries/blocks 为孤儿。
- **已做**：移植 `PdfPreview.vue`（pdfjs getDocument＋canvas/TextLayer＋页码/缩放/适应宽度，
  133 行）与 `OfficePreview.vue`（docx→readWordDocument／xlsx→readWorkbook，sandbox iframe＋
  工作表 tablist＋缩放，83 行），对齐 React 基准。`DocumentPreview.vue` 加 pdf/office 分派支路
  ＋基准的 office-off 设置门（`useExternal(onOfficePreviewChange, previewsOffice)`，关闭时显示
  open-external/go-settings 卡，与 oracle DocumentPreview.tsx:46/56-60 一致）。样式并入
  `src/design/document.css`（簇既有 design.css @import 模式）；i18n 键补齐；`orphan.test.ts`
  移除 officeSheet/officeWord 两条（查看器经 DocumentPreview reached）。deps（pdfjs-dist/
  docx-preview/exceljs/ssf）与解析助手早已在；`vite.config.ts` 的 pdfAssets() 插件构建期 emit
  dist/pdfjs/{cmaps,standard_fonts,wasm}＋worker，资产真实解析（优于 oracle 无资产）。
- **实测（主 Agent 重跑，非子 Agent 自报）**：document 簇＋orphan＋i18n msg.test 7 文件/67 测试绿；
  全量 `vitest` **298 文件/2196 测试**绿；`vue-tsc -b --force` EXIT 0；`vite build` EXIT 0（emit pdfjs）。
  子 Agent 误报「zero i18n edits」（实为新增键）， outcome 正确，已核实键存在且 msg.test 绿。
- **剩余孤儿**：`deliveries.ts`/`ui/blocks.ts` 是另一条 delivery-card 链（查看器不 import），
  不在 B9 范围，仍记 KNOWN_ORPHANS。pptx 仍 unsupported（oracle 亦无 pptx 解析，基准一致）。
- **状态**：done（PDF/Office 查看器），2026-11-18 波次 9。

### B10 —— agent-runs 工作台子系统：核心已移植，余项有界裁决

- **已落地（波次 11，done）**：agent-runs 工作台核心整条移植并接线——
  `ui/useAgentRequest.ts`（open/openJob/consumed，shallowRef 按身份清除）、
  `ui/AgentRunView.vue`（轮询 `port.subagentTranscript(ref,after)`、任务抬头、运行态/停止/只读注记）、
  `useAgentTabs`（agent/job Surface 进工作台标签条）被 `WorkbenchPanel.vue` 消费，
  `Pane.vue` `provide(AgentRunOpener, agentRuns.open)` 并 `reveal=openColumn("files")`。
  `SubagentGroup.vue` 的 AgentRow 打开按钮经 `inject(AgentRunOpener)` 到达，
  live `Transcript.vue` 已用 `workLanes` 分组（B10a 早已落地）。`useAgentTabs`/
  `useAgentRequest`/`AgentRunView`/`SubagentGroup` 转 reached，从 `KNOWN_ORPHANS` 移除
  `useAgentTabs`（debt 84→83）。组件测 17（agent-run-tab 9 + useAgentRequest 8）＋
  工作台接线测 1，主 Agent 重跑全绿（vue-tsc 0；定向 5 文件/30 测试）。
- **余项（精确缺口，有界裁决见 D15，非半措施）**：
  - **B10c job 查看器**：`useAgentRequest.openJob` 已落地但 Vue 无发起 job-open 的调用点
    （无后台 job Surface 来源），`JobRunView` 未移植。runs 路径是 live 的那条；job 查看器
    待一个 job-opening 调用点才有意义，裁决移出当前范围。
  - **B10d 「回到分组」onShowCall**：依赖 Transcript 的 focus-scroll 子系统
    （land/revealNested/scrollIntoView）。Vue `Transcript.vue` 声明 `focus` prop 但从不读它
    （inert），故 back-to-call 无法生效。`AgentRunView` 以 `onShowCall` 可选门控——缺则
    **不画**返回按钮（诚实缺席，非 inert 按钮），Pane 暂不供给。裁决待 focus-scroll 移植。
  - **openCards.ts**：另一条 card-opening 链（非 agent-runs 工作台），仍孤儿，本波次未触。
- **状态**：partial —— 核心 done（2026-11-19）；B10c/B10d/openCards 有界裁决（D15），跳过。

### B11 —— Files rail 面板已挂载；Plan 面板仍未移植

- **已落地（波次 11，done）**：`Metrics.vue` 的 `ORDER` 增 `files` 段并 `import Files`、
  渲染 `<Files :changes :tree :yolo>`；`Pane.vue` 派生 `rail = railOf(items,executions,
  subagentPhase)`（`panels/derive`）并把 `rail.changes`/`tree`/`yolo` 喂给 analysis 视图的
  Metrics。`panels/Files.vue`/`panels/derive.ts` 转 reached，从 `KNOWN_ORPHANS` 移除。
  「工作树改动」rail 面板遂在分析页显示，git 权威 tree 过滤已撤销改动。
- **仍缺（精确缺口）**：`Plan` 面板（React `ui/Plan.tsx`）Vue 未落地，Metrics ORDER 无
  `plan` 段；`useConversationFiles`（filelinks/grantedfiles/WorkbenchFileMenu/Markdown
  LocalRefs 整条文件链接→工作台预览链）未移植，Files 的 `onOpen`(openPreview) 未接该链。
- **状态**：partial —— Files rail done（2026-11-19）；Plan 面板与 conversation-files 链
  blocked（依赖整条子系统），跳过。

### B12 —— pane-opening 核心已落地；sidebar-open 内联，并发 ticket 排序未移植

- **已落地（波次 11，done）**：`App.vue` 现有**每 pane 一份** `viewRequests =
  ref<Record<string,{to,n}>>`（:135）＋ `requestView/showView`（:395-399）＋ `openPane`
  （:332，busy 守卫的 `hub.open`→refresh→active），传给当前 pane（:651）并被
  `useBranchSession`（:345）消费。这条核心解了 B14 的 branch 前置（branch 经 openPane
  落在源 root 旁）。**sidebar-open-to-flow 行为已内联**在 `openSession`（:318-328）：
  重点击已开会话 `active=held.id` + `requestView(held.id,"flow")`，正是 useSidebarOpen
  的语义——按 D13 冗余内联，`sidebaropen.ts` 保留孤儿（功能已在，非缺口）。
- **仍缺（精确缺口）**：`sessionswitch.ts`（useSessionSwitch 的并发开 ticket 排序
  begin/current/landed/end/cancel）未移植——Vue 用串行 busy 守卫 openPane，无并发 ticket
  消费点，仍孤儿。`useOpenPane.ts`/`useOpenRemotePane.ts` 助手未抽出（openPane/openRemote
  内联在 App）。
- **状态**：partial —— 核心 + sidebar-open-to-flow done（2026-11-19，后者内联 D13）；
  并发 ticket 排序 blocked（Vue 串行开 pane，无消费点），有界裁决见 D15，跳过。

### B13 —— 工作台浏览器标签改名：架构分歧，已裁决移出 Vue 范围

- **范围**：`ui/useTabNames.ts`（names/renaming/setRenaming/labelFor/nameTab）仍是孤儿。
  React `WorkbenchPanel.tsx:183` `useTabNames((surface) => labelOf(surface, hosts))`，
  标签条在 `renaming === keyOf(surface)` 时画 `input.workbench-tab-in`（:518-534），
  双击/F2 起改名（:550/554），**均以 `surface.kind === "browser"` 为前提**。
- **被什么挡住**：Vue `WorkbenchPanel.vue:219-223` 的 `surfaces` 只产 `changes`/`file`/
  `terminal` 三种，**没有 browser 面**（React :314-319 有 `...tabs.map(tab => ({kind:"browser",tab}))`，
  Vue 无此 `tabs` 来源，grep `kind:"browser"`/`pages` 于该文件为空）。浏览器视图在 Vue 走
  App/Pane 的停靠浏览器列（D6/D12），不进工作台标签条。因此即便接上 useTabNames，
  改名 UI 也永远没有 browser 标签可作用 —— 是死代码，不是缺口修复。
  `.workbench-tab-in` 样式虽早已在 `studio.css:3583`，但无消费面。
  同簇的 `BrowserTabMenu`/`tabMenu`（React :787）在 Vue 亦无。
- **后果**：useTabNames.ts 保留孤儿（类型检查通过）。工作台标签不支持双击/F2 改名。
- **裁决（D15，终态）**：浏览器在 Vue 走停靠浏览器列（D6/D12），**不进工作台标签条**，
  这是刻意的架构分歧而非待补缺口。改名 browser-only，故 useTabNames 在 Vue 工作台
  **结构性不可达**——接上是死代码。裁决「工作台浏览器标签改名」**移出 Vue 范围**，
  useTabNames.ts 按 D13 保留孤儿记录（功能无消费面，非半措施）。波次 11 的 agent/job
  Surface 进标签条不改变此结论（agent/job 非 browser，改名仍无作用面）。
- **状态**：adjudicated out-of-scope（D15，2026-11-19）——架构分歧，保留孤儿，不接线。

### B14 —— 会话菜单：可达切片＋运行时动作已落地，余项有界裁决

- **已落地（波次 10，done）**：`sidebar/SessionMenu.vue` 扩为四组菜单，新增
  **pin / move(up·down) / auto-name / mark-read / copy-info / export(md·json)**。
  所有权镜像 React `Workspaces.tsx`：这些变更由 `Sidebar.vue` 自跑（`hub` + `reload` +
  `onError`），`App.vue` 仅供 `reload: refresh`。四个原孤儿（`sessionExport.ts`、
  `sessionExportMarkdown.ts`、`sessionInfo.ts`、`useSessionMove.ts`）被接线到达，
  从 `KNOWN_ORPHANS` 移除（debt 92→88）。CSS/icon/i18n 早已齐，无新增。
- **take 顺序根因修复**：组件 `take` 原先先 `emit("close")` 再跑动作，而 Sidebar 的
  `@close` 清空 `menuAt`，致动作读到空目标（rename/archive/delete 同样受影响的潜在 bug，
  此前无侧栏级菜单测试故未暴露）。改为先跑动作再 announce close，对齐 React（父级先捕获
  session 再关闭）。owning layer 一处修复，惠及所有项。
- **运行时动作已落地（波次 11，done）**：`onPause / onCloseConversation / onBranch /
  archive-live-disable` 现经**每会话 live 状态**门控并接线——B12-core 引入的
  `App.vue runs = ref<Record<string,{live}>>` 供 `Sidebar.vue menuLive`（`props.runs[id].live`），
  `SessionMenu.vue` 以 `held && live` 画 pause/branch、close 在 live 时 `:disabled` 并附
  「先暂停」说明（`session.running_pause_it_before_closing`）。Sidebar 接
  `:on-pause/:on-close-conversation/:on-branch`（:489-491）经 `take(pauseTarget/closeTarget/
  branchTarget)` 路由到 `onPauseSession/onCloseSession/onBranchSession`；branch 经
  `useBranchSession`→`openPane` 落在源 root 旁（B12 核心解了此前置）。
- **仍缺（精确架构原因，非半措施，有界裁决见 D15）**：
  - `kept-versions`（保留版本展开）：依赖 versions+copies 分散 UI，Vue 未移植。
  - `onMoveTo`（移动到别的项目）：Vue 无 `useSessionMoveTo` 助手＋项目选择对话框。
  这些项在组件里以可选回调门控（缺则不画），是诚实缺席而非半接线。
- **验证（主 Agent 重跑，波次 11）**：vue-tsc -b --force EXIT 0；全套 vitest **304 文件/
  2250 测试**绿；vite build EXIT 0（3292 模块）；orphan debt **83**。波次 10 的可达切片
  验证（SessionMenu 11 + sidebarSessionMenu 9 + sidebarReachable 6 + orphan 3）仍绿。
- **解锁条件（余项）**：移植 kept-versions 子系统、`useSessionMoveTo`＋项目选择对话框；
  或经裁决把这些项移出 Vue 范围（D15 已记）。
- **状态**：partial —— 可达切片（波次 10）＋运行时动作（波次 11）done；kept-versions/
  onMoveTo 有界裁决（D15），跳过。

### B15 —— Rust 内核未对等，不能从 Go cutover；部分供应链门本机未装

- **范围**：D14 采纳的上游 B 内核（M2）已在 trunk 构建/测试全绿，但它是**进行中**
  的重写，不是可替换 Go 的成品。
- **被什么挡住**：
  - 账本 563 行＝69 done／1 wip／**493 todo**（~12%）。质量集中在 state/contract/core；
    agent/tools/provider/serve/safety 仍是 8 行 skeleton。
  - `apps/reasonix-studio-host` 是 stub：每次调用 exit 2，自报「does not implement
    the Studio host yet」（HOST 行在 M4）。`apps/reasonix` 仅 version flag（CLI-001..003）。
  - 故 Tauri 壳仍 spawn **Go** host（`src-tauri/bin/reasonix-studio-host.exe`），
    Rust host 未接线，无 cutover。
- **供应链门 skipped**：`cargo-deny`/`cargo-nextest`/`cargo-machete` 本机未安装
  （`cargo deny --version` → no such command）。已跑的门 fmt/clippy/test/xtask parity/
  xtask deps 全 0；`deny.toml` 策略随 B 采纳，但 `cargo deny check` 未执行。
- **工具链**：根 `rust-toolchain.toml` pin 1.99.1（B 的 MSRV，本机已装）；壳无独立 pin，
  rust-version=1.90 ≤ 1.99.1、edition 2024 兼容，`cargo check` 0（3m59s 重编译）。
- **网络**：crates.io 与 rsproxy CDN 本机 DNS 间歇失败；USTC sparse 镜像配在
  `~/.cargo/config.toml`（机器级，**不提交进仓库**）。换机需重配镜像或恢复 DNS。
- **解锁条件**：按 B 的 M3–M6 推进 parity（serve/provider/tools/agent/safety），
  账本 todo 收敛、golden fixtures 对等、host 实现 HOST 行后，经 parity 签收＋一次真实
  端到端任务才 cutover（RUST_MIGRATION 决策 5）。安装 cargo-deny 等补跑供应链门。
- **状态**：blocked（cutover）；内核合并本身已完成并验证，2026-11-18 记录。

### B16 —— pinia 死依赖无法最小化删除：pnpm-lock 与 package.json 长期脱钩

- **试了什么**：D5 脚注的清理 —— 从 `desktop-tauri/package.json` 删除无 import 的
  `pinia ^3.0.0`，并同步 lockfile。
- **被什么挡住**：`pnpm-lock.yaml` 与 `package.json` 严重脱钩。HEAD 的 lock `importers`
  只声明 4 个依赖（`@tauri-apps/api`、`naive-ui`、`pinia`、`vue`），而 package.json 已演化到
  ~50 个（codemirror/a2ui/pdfjs 等，且**不含 naive-ui**）。`pnpm install --lockfile-only`
  会把整张 lock 重新解析对齐 package.json：实测 diff **4663 增／676 删**，其中
  (1) 补全 importers；(2) 剪掉 package.json 未声明的 `naive-ui` 及其整条依赖树
  （css-render/@emotion/async-validator/date-fns/lodash/highlight.js 等）；(3) 把 `^` 范围
  重新解析到较新补丁版（如 `@babel/parser` 8.1.0→8.1.2）。这远超「删一个死依赖」，
  属一次依赖图变更，违反最小 diff 与「不擅自升级依赖」。
- **实测安全性**：`grep -rn naive-ui src/` 与 `grep -rn pinia src/` 均为空 —— 二者都无
  import；node_modules 里 naive-ui 是多余物、pinia 是死重量。故重解析对运行中的 app 无害，
  但补丁版 bump 与 naive-ui 剪枝仍是 D5 之外的改动，且 `--lockfile-only` 不动 node_modules，
  无法在不重装的前提下真实验证新 lock 的 frozen 安装。
- **恢复**：package.json 的 pinia 行已还原（净零改动）；lockfile 已恢复到 HEAD 基线
  （`git show HEAD:… > …`，diff 归零）。**留痕**：恢复前的 worktree lock 是波次间累积的
  脱钩产物，已被本次重解析覆盖、不可逐字节复原；HEAD 是可恢复的干净基线，node_modules
  （实际运行依据）全程未动，故运行态不受影响。
- **解锁条件**：把 lock↔package.json 的重对齐作为**独立、经评审的任务**做（决定 naive-ui
  去留、接受补丁版 bump、`pnpm install --frozen-lockfile` 真实回归），再在其上删 pinia。
  不把它 bundled 进死重量清理。
- **状态**：skipped（D5 清理搁置），lock 脱钩记为待评审的独立缺陷，2026-11-18 记录。

## Closed

### C1（原 B0）—— 「环境无子 Agent」的结论已过时

- **原记录**：波次 1 时每次派发都被拒，错误是
  `Agent type 'general-purpose' is disabled via disallowedTools configuration.`，
  `Explore` 与无类型调用同样被拒。据此认定本机不支持子 Agent，波次 2–5 串行。
- **修法 / 现状**：该结论**已不成立**。当前主机实测支持最多 **8 路并行子 Agent**，
  波次 2–5 的模块级任务确实是并行派发的。串行降级记录作废，`agents.md` 的
  并行度表改为实际值。
- **遗留**：并行可用性受 B7 的 DNS 不稳定影响 —— 能并行，但会因解析失败而重试。

### C2（原 B5）—— `pnpm tauri build` 曾被配置挡住

- **原记录**：`tauri.conf.json` 里 `"bundle": { "active": false }`，且本机开不了
  WebView2 窗口，所以无法产出安装包。
- **修法 / 现状**：`bundle.active` 保持 `false` 是刻意的默认（开发期不产出安装包），
  但 CLI 可以在命令行显式要求打包，覆盖配置。
- **实测**：`tauri build --bundles nsis` → 退出码 0，产出
  `bundle/nsis/Reasonix Studio_0.1.0_x64-setup.exe`（2 677 320 字节，2.55 MiB，
  时间戳 22:28）。不带 `--bundles` 的 `tauri build` 同样退出 0，只编译
  `reasonix-studio.exe`，不做封装 —— 与 `active: false` 一致。
- **遗留**：安装包**没有**在干净机器上安装并启动验证过（受 B1 限制）。所以
  「打包可用」是 `done`，「安装后可运行」是未验证。

### C3 —— 五个 `.vue` 扩展名错误

- **症状**：`vite build` 直接失败。
- **原因**：五个文件是**裸 TS 源码** —— 只有命名导出，既无 `<script>` 块也无
  `<template>` 块，却用了 `.vue` 扩展名。`.vue` 会交给 SFC 解析器，而 SFC 要求
  至少有 `<script>` 或 `<template>` 之一。
- **涉及文件**：`ui/SettingsLeave`、`ui/panels/kit`、`ui/chart/plots`、
  `ui/GitPushCard`、`ui/RuntimeReload`。
- **实测该规则**：用 `vue/compiler-sfc` 的 `parse` 直探，裸 TS 文本作为 `.vue` →
  `At least one <template> or <script> is required in a single file component.`。
  只有 `<script>` 而无 `<template>` 时 `compileScript` 反而正常通过。
- **所以失败点是「缺少块」而不是「缺少 template」**。这个区别重要，因为
  `ui/vendormarks.vue`、`ui/SettingsPageStyle.vue` 同样没有 `<template>`，
  却因为有 `<script>` 而合法。
- **修法**：全部改名为 `.ts`，同步改 import 路径。实测这五个现在都以 `.ts` 存在
  （`SettingsLeave.ts`、`panels/kit.ts`、`chart/plots.ts`、`GitPushCard.ts`、
  `RuntimeReload.ts`），五个 `.vue` 版本均不存在；`vite build` 退出 0。
- **纪律**：写成 `decisions.md` D10。

### C4 —— `SettingsPageStyle` 多出的 `foreign` CSS 规则

- **症状**：设置中心里 model 页的 providers 块、hooks 页的整页被藏掉，
  界面一片空白，但 typecheck 与单测全绿。
- **原因**：Vue 侧 `SettingsPageStyle.vue` 比 React 基准
  `SettingsPageStyle.tsx` 多出一条 `foreign` 规则，把「被折叠进本页、但 section
  归属不同」的块也一并 `display:none` 了。
- **修法**：按基准移除该规则。实测两个文件现在都**不含** `foreign`
  （`grep -q foreign` 两边均为否），`SettingsPageStyle.vue` 53 行。

### C5 —— `App.vue` 三处接线错误

- **症状一：`onSettings` 丢掉 anchor 参数。**
  `Settings.vue` 支持 `section:anchor` 形式的落点（如 `memory:memory`、
  `versions:logs`），但 `App.vue` 把回调参数扔了，于是「跳到某个设置项」永远
  落在页首。
- **修法一**：保留 `at?: string` 并写入 `settingsAt`。实测有 `settingsAt` 的
  `ref<string | undefined>(undefined)` 声明、`openSettings(at)` 调用，以及传给
  `Settings` 的 `at: settingsAt.value`。
- **症状二：`Chrome` 被传 `port: null`。** 查找与视图切换控件因此不渲染。
- **修法二**：传 `port: activePort.value`。实测 `h(Chrome, { port: activePort.value })`。
- **症状三：`Nav.onFiles` 语义漂移。** 曾被当成「跳到某个路由」，而 React 基准里
  它是一次**请求脉冲**：`App.tsx` 的 `openFiles` 把 `filesRequests[active]` 加一，
  由当前 pane 收到后打开自己的 files 栏。
- **修法三**：`App.vue` 改为 `onFiles: () => { filesPulse.value += 1; }`；
  `Pane.vue` 用 `watch(() => props.filesPulse, ...)` 在 `props.active` 时
  `openColumn("files")`。与 `terminalPulse` / `openTerminal` 形状一致。
- **仍未对齐，如实记录**：React 的 `openFiles` 还带两个副作用 —— 没有活动会话时
  报 `app.open_a_session_first_to`、以及先 `hidePrefs()`。Vue 侧这两个都没有，
  `active` 为假时只是静默不开栏。这是一处尚未对齐的行为，不是已完成项。

### C6 —— `mono` 守卫的 MIXED 集合是基准的不完整子集

- **症状**：`styles/mono.test.ts` 的 `MIXED`（同时承载散文与数据的等宽类）
  只有 19 项，而 React 基准是 27 项。
- **修法**：守卫改为真的读 Vue 树 —— `import.meta.glob("../**/*.vue", ...)` 取源码，
  用 `vue/compiler-sfc` 的 `parse` 按模板 AST 判定散文与数据子节点；并按基准重新
  校准集合，方向是「记录真实债务」而不是「让测试变绿」。
- **准确到实测的说法（与任务书描述有出入，如实标出）**：任务书说「补齐后守卫
  恢复」。实测当前 `MIXED` 是 **19** 项，基准 **27** 项，且 Vue 的 19 项全部包含
  在基准的 27 项里（脚本比对字面量，差集为空）。
  也就是说：**集合并没有被补成 27 项**。
- **差集**：`cost`、`cur`、`delta`、`meta`、`path`、`pt`、`rmtcand`、`uspan`
  在基准里有、Vue 侧没有。
- **为什么这不是漏改**：守卫第二条测试 `does not grow the set of classes carrying
  both prose and data` 是绿的，而它断言的是更强的命题 —— 「Vue 树里不存在任何
  不在 `MIXED` 里、却同时渲染散文与数据的等宽类」。既然它绿，这 8 项在 Vue 树里
  就都不构成混合盒；写进集合属于虚增债务。
- **所以状态是**：守卫从「空转」恢复到「真的在读 Vue 树并断言」，
  `vitest run src/styles/mono.test.ts` → 3 passed；与基准的 8 项差异保留为
  **已知差异**，记在这里而不是抹平。
- **未验证**：无法确知它当初是怎么失败的 —— 没有留下失败输出。

### C7 —— `orphan.test.ts` 曾是永远绿灯的空壳

- **症状**：守卫的 glob 只找 `.tsx` 文件，而 Vue 树里 `.tsx` 数量为 **0**。
  于是它什么也没读到、什么也没断言，却对着 321 个组件报绿 —— 正是它存在意义的
  反面。
- **修法**：重写为真正遍历 Vue 树的守卫。
- **现在的样子**：`import.meta.glob(["./**/*.{ts,vue}", "!./**/*.test.*"])` 读源码
  文本；按四种边做可达性遍历 —— 字面 import / export、`import()` 分块、worker 的
  `new URL(..., import.meta.url)`、以及 `import.meta.glob` 本身；入口显式声明为
  `["src/main.ts", "src/testsetup.ts"]`，不靠文件名推断。
- **三条测试**：不许新增未登记孤儿 / 登记的必须仍是孤儿（债务缩小就要删条目）/
  真的读到了树。
- **实测**：`vitest run src/orphan.test.ts` → 3 passed；债务条数 129
  （`grep -c '^  \["src/'`）。
- **纪律**：写成 `decisions.md` D11。
