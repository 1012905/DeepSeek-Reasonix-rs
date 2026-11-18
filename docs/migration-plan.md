---
owner: "@SivanCola"
backup: "@esengine"
status: active
reviewed: 2026-11-17
---

# 迁移计划

目标：在主干上合并成一个项目 —— Tauri 2 桌面壳、Vue 3 + TypeScript + Vite 前端、
Go 内核继续作为后端；原 Electron/React 应用作为功能验收基准。

塑造本计划的裁决在 `decisions.md`；实时状态在 `migration-state.md`；阻塞在
`blockers.md`；子 Agent 分工在 `agents.md`。

## 交付物与验收口径

| # | 交付物 | 当前 | 缺什么 |
|---|---|---|---|
| 1 | 完成桌面 UI 的 Vue 3 移植 | **partial** | 屏幕都在、都在界面里；94 条可达性债务经全量复核为 34 冗余内联（D13）+ 19 测试夹具 + 41 子系统阻塞（B9–B14 等），干净接线已尽 |
| 2 | Tauri 壳与 Electron 主进程能力对等 | **partial** | 契约层已注入 33/34（`NAMES` 31 命令）；残差为真实窗口验证（B1）与两处设计差异，见下表 |
| 3 | webview 与内核之间只有一层契约 | **done** | — |
| 4 | 每层的验证证据（含命令与结果） | **done** | 全部在 `migration-state.md` |
| 5 | 打包产出安装包 | **done** | 2.55 MiB NSIS 安装包；未在干净机器安装验证 |
| 6 | 完成度对照表、阻塞表、退役计划 | **partial** | 退役计划只有 D4 的原则，没有按模块列出的顺序 |

第 3 条的判据：`port/hub.ts` + `port/port.ts` 是唯一线缆面。实测 `port/kernel.ts` 与
`stores/kernel.ts` 已删（`git status` 记为 `D`），无第二份客户端实现。

## 已完成且不得重做

- **Go 内核**：完整、未改。`go build ./...` 与 `go vet ./...` 均绿，`git status` 对
  `internal/`、`cmd/`、`go.mod` 零改动。仍是实际后端，直到 Rust 对等签收（B15）。
- **Rust 内核（D14 采纳，上游 B M2）**：`crates/`（17）＋`apps/`＋`xtask/` 已并入主干，
  取代 A 谱系停放骨架；A 的 `contract` eventwire/pricing golden 切片保留。`cargo test
  --workspace --all-features` 572 绿、fmt/clippy/xtask parity（563 行）/deps 全 0。
  账本 69/563 done，host 仍 stub，**未 cutover**（B15）。两栈共存至对等（RUST_MIGRATION 决策 5）。
- **Tauri 壳**：起 Go host、把内核 SSE 桥到 webview、加载内核对外的页面、31 条命令
  （含窗口截图、图形状态、全局快捷键、进程内存、文本右键菜单、原生拖放，以及内嵌
  agent-browser 子 WebView 的显隐/导航/控制/登录/证书/挑选）。`cargo check`、
  `cargo clippy --all-targets -D warnings`、`cargo fmt --check`、`cargo test`（90）均绿。
- **Vue 端口**：317 `.vue`、770 `.ts`、286 测试文件 / 2148 测试。核心对话流（发送、
  流式、工具卡片、审批、ask、终答）端到端可用。

## 波次

### 波次 1 —— 盘点与裁决（完成）

三树盘点、主干裁决、A/B 的吸收裁决、基线验证。产出 `decisions.md`、
`migration-state.md`、本文件。

A 与主干的实测 diff（命令与展开规则见 `migration-state.md`），按排除集分档：

| 排除集 | 仅 A 有 | 仅主干有 | 内容不同 |
|---|---|---|---|
| `node_modules` / `.git` / `target` | 5 | 2023 | 71 |
| 再排 `dist`，展开目录 | 5 | 1285 | 71 |
| 再排 `vendor` / `.reasonix` / `gen` / `public` | 5 | 1262 | 71 |
| 只数源码扩展名 | 5 | 1244 | 71 |

结论：A 的 `desktop-tauri` 是主干自身工作的更早状态 —— 其 `src` 只有 `App.vue`、
`main.ts`、`port/`、`stores/` 共 5 个文件，主干同名目录有 1210 个。整体吸收即回归，
所以不吸收。

B 是 Go→Rust 内核重写（25 个 `.rs`、1585 行、ledger 19 行里 6 行 todo），方向与本
迁移相反，不吸收；只归档 ADR 与 ledger 格式（归档目录尚未创建，见 B6）。

> **已被 D14 取代（波次 8）**：用户裁决采纳 B 上游最新版（Gitee `9b3d7ba`，M2）的
> 内核重写。本地陈旧 B（`87bfa14`，M0）仍不吸收；上游 B 的 crates/apps/xtask 已并入，
> B 的 ADR/ledger 采纳为 live 文档（B6 归档冲突就此消解）。详见 `decisions.md` D14。

### 波次 2 —— 缺失的屏幕（并行，完成）

文件所有权：每个子 Agent 只创建自己的新文件。`App.vue`、`Pane.vue`、`main.ts`、
`src-tauri/**`、`port/**`、`i18n/**` 归主 Agent，接线在波次之间做。

| 任务 | 归属范围（创建） | 基准 |
|---|---|---|
| W2-SETTINGS | `ui/Settings.vue` + `ui/settings/{registry.ts,sections/,parts/}` | `Settings.tsx`(595)、`SettingsNav.tsx`(98) |
| W2-SIDEBAR | `ui/Sidebar.vue` + `ui/sidebar/` | `Sidebar.tsx`(399) |
| W2-CHROME | `ui/Chrome.vue` + `ui/chrome/` | `Chrome.tsx`(169) 等 5 个 |
| W2-ONBOARD | `ui/Onboarding.vue`、`ui/Welcome.vue`、`ui/OnboardingProject.vue` | `Onboarding.tsx`(391)、`Welcome.tsx` |
| W2-PANELS | `ui/panels/*.vue` | 同名 `.tsx` |
| W2-BROWSER | `ui/browser/*` | `BrowserPanel`、`BrowserLogin`、`BrowserTools`、`BrowserTabMenu` |

实测产出：

| 族 | 产出 |
|---|---|
| 设置中心 | `sections/` 20 个 + `parts/` 48 个 + `registry.ts`；`Settings.vue`(286)、`SettingsNav.vue`(61) |
| Sidebar | `Sidebar.vue`(435) + `ui/sidebar/` 12 项 |
| Chrome | `Chrome.vue`(159) + `ui/chrome/` 4 个 |
| Onboarding | `Onboarding.vue`(535) + `Welcome.vue` + `OnboardingProject.vue` + `ui/onboarding/` |
| panels | `ui/panels/` 9 个面板 `.vue` + `JobName.vue` |
| browser | `ui/browser/` 7 个 `.vue` |

20 个 section 文件与 `prefsnav.ts` 的 `Section` 联合类型 20 个成员集合相同 —— 实测，
不是推断。

### 波次 3 —— 接线与集成（完成）

- `App.vue` 挂载 `Chrome` / `Settings` / `Sidebar` / `Onboarding`：实测有
  `h(Chrome,…)`、`h(Sidebar,…)`、`settingsOpen ? h(Settings,…)`、
  `guide ? h(Onboarding,…)`。
- `Pane.vue` 接入真实 `view` / `context` / `mcp` / `surfaces`：读 `props.port.context()`、
  `props.port.mcp()`、`s.panels.length + s.views.length`。
- 同波次修掉三条接线 bug（详见 `blockers.md` C5）：`onSettings` 丢 anchor、
  `Chrome` 收到 `port: null`、`Nav.onFiles` 语义漂移。

### 波次 4 —— 面板与工作台（并行，完成）

| 族 | 实测产出 |
|---|---|
| BrowserPanel 族 | `ui/browser/` 7 个 `.vue` + 2 个测试 |
| Remotes 族 | `ui/remote/` 6 个 `.vue` + `hostbook.ts` + 1 个测试 |
| panels 族 + Metrics | `ui/panels/` 9 个面板 + `kit.ts`、`derive.ts`、`jobElapsed.ts`；`ui/Metrics.vue` |
| Toast / Find / PaneTabs | `ui/ToastStack.vue`、`ui/ToastRow.vue`、`ui/find/Find.vue`、`ui/PaneTabs.vue` |
| Workbench（5）+ Terminal（5） | `ui/workbench/` 5 个、`ui/terminal/` 5 个 |

### 波次 5 —— 接线与收口（完成）

把波次 4 的成品接进界面：

- `Metrics` / `BrowserPanel` / `Find` / `WorkbenchPanel` / `TerminalView` 接进 `Pane.vue`。
- `ToastStack` / `PaneTabs` / `Remotes` / `RemoteAsk` 接进 `App.vue`。
- 修掉五个 `.vue` 扩展名错误、`SettingsPageStyle` 的 `foreign` 规则、`App.vue`
  三处接线、`mono` 守卫集合、`orphan` 守卫空壳（见 `blockers.md` Closed）。

### 波次 6 —— ShellBridge 对等与内嵌浏览器（完成，契约层）

主 Agent 串行执行（本波次子 Agent 全程禁用）。范围：补齐壳命令对等、落地原生
agent-browser 子 WebView、把同步 `pathForFile` 改为原生拖放路径事件。

- `NAMES` 16 → 31；`build.rs` ACL 与 `bridge.rs` 精确名单守卫同步，二者绿。
- 新增 `capture/graphics/shortcut/procmem/textmenu/dropwin` 六个系统能力模块与
  `browser/` 子 WebView 模块（policy/pick/protocol/relay/views/certs）。
- `bridge.rs` 注入 33/34 个 `ShellBridge` 方法；`pathForFile` 刻意移除，由 `onDropPaths`
  取代；浏览器登录/加载状态走 event 插件。
- 子 WebView 隔离：按 target 持久化存储、内核 origin 与壳桥接不可达、凭据 cookie 只设
  主 webview、导航/弹窗/下载/权限拒绝。无平台等价物显式 unsupported。
- 修两处前端测试（`host_drop.test.ts` 闭包窄化、`firstrun.test.ts` 的 `shellPicks`
  展开丢原型方法）。
- 验证：vue-tsc 0、vitest 286/2148、vite build 3225 模块、go build/vet 0、
  cargo fmt/clippy/test（90）绿。真实窗口验证仍受 B1 阻塞，macOS/Linux 标 partial。

## 完成度对照表

状态用 `done` / `partial` / `skipped`。任何 `partial` 都写出缺什么，不允许只写状态。

| 原模块 | 目标实现 | 来源 | 状态 | 缺什么 |
|---|---|---|---|---|
| Go 内核 `internal/**`、`cmd/**` | 保持不变 | 主干（原有） | done | — |
| Rust 内核重写 | `crates/`＋`apps/`＋`xtask/`（上游 B M2，`9b3d7ba`） | D14 采纳 | partial | 账本 69/563 done；`cargo test` 572 绿、fmt/clippy/xtask 全 0；host 仍 stub，未 cutover（B15）；cargo-deny 等未装 |
| 核心对话流 | `ui/Pane.vue`(491) + `ui/cards/` 32 个 `.vue` | 主干（原有） | done | 等待提示 `Await.vue` 已按基准接 retry（波次 7）；`SubagentGroup.vue`/`openCards.ts` 依赖 agent-runs 工作台（B10） |
| 设置中心 | `Settings.vue`(286) + `sections/` 20 + `parts/` 48 + `registry.ts` | 波次 2 | done | 运行时重载按钮已接（波次 7）；`AuthorLine.vue` 冗余内联于 versions.vue（D13） |
| Sidebar | `Sidebar.vue`(435) + `ui/sidebar/` 12 项 | 波次 2 | partial | 会话菜单只 rename/archive/delete；export/copy-info/move/move-to 子系统未移植（B14） |
| Chrome | `Chrome.vue`(159) + `ui/chrome/` 4 个 | 波次 2 | done | 三处接线 bug 已修（C5） |
| Onboarding | `Onboarding.vue`(535) + `Welcome.vue` + `OnboardingProject.vue` | 波次 2 | done | `Welcome.vue` 已接线（波次 7） |
| Panels 本体 | `ui/panels/` 9 个 `.vue` | 波次 2 | done | 8 个由 `Metrics.vue` 挂载 |
| Metrics 侧栏的 Files 面板 | `ui/panels/Files.vue` | 波次 2 | partial | 依赖 conversation-files/pane-derive 子系统（B11） |
| BrowserPanel 族 | `ui/browser/` 7 个 `.vue` | 波次 4 | done | — |
| Remotes 族 | `ui/remote/` 6 个 `.vue` | 波次 4 | partial | `RemoteProjectMenu`/`RemoteSessionRow` 依赖远程项目/会话 rail 子系统（未移植） |
| Toast / Find / PaneTabs | 4 个 `.vue` | 波次 4 | done | `usefind.ts`/`usefindask.ts` 功能已内联于 Find.vue/App.vue（D13 冗余） |
| Workbench（5） | `ui/workbench/` 5 个 `.vue` | 波次 4 | done | `WorkbenchLauncher.vue` 冗余内联（D13）；浏览器标签改名 `useTabNames.ts` 无 browser 面（B13） |
| Terminal（5） | `ui/terminal/` 5 个 `.vue` | 波次 4 | done | — |
| 图表渲染器 | `ui/chart/` 6 个模块 | 波次 2 | done | ChartCard/Boundary 已接进 Transcript（波次 7）；`fixtures.ts` 为测试夹具 |
| 文档预览 | `ui/document/` 3 个 `.vue` + 6 个 `.ts` | 波次 2 | partial | 表格/HTML/图片查看器已接（波次 7）；PDF/Office 查看器未移植（B9） |
| 扩展面板 surface | `ui/surface/` 3 个 `.vue` + 1 个 `.ts` | 波次 2 | partial | `usesurfaceslots` 冗余内联于 SlottedView（D13）；rich/Workspace、media-context 子系统未移植 |
| Electron 主进程 IPC | `commands.rs`（31 条 `NAMES`）+ `browser/` 模块 | 波次 6 | partial | 契约层 33/34 已注入；真实窗口未验证（B1）；残差见下节 |
| webview↔内核契约 | `port/hub.ts` + `port/port.ts` | 主干（原有） | done | — |
| 打包 | `tauri build --bundles nsis` | 波次 5 实测 | done | 未在干净机器安装验证 |

## IPC 对等：34 声明 / 33 注入 / 31 命令

波次 6 实测：`port/host.ts` 的 `ShellBridge` 接口声明 **34** 个方法
（`sed -n '/^interface ShellBridge/,/^}/p' | grep -cE '^  [a-zA-Z]+\??\('`）。
`src-tauri/src/bridge.rs` 注入 `window.reasonixHost` 实现其中 **33** 个（逐名 grep 核对）。
`commands.rs::NAMES` 是 **31** 条 invoke 命令；另有 3 个订阅方法
（`onDropPaths`、`onBrowserLoadState`、`onBrowserLogin`）走 event 插件而非命令，
所以 31 命令 + 3 事件订阅 = 34 减去刻意移除的 `pathForFile`。

原有的 16 条（Electron 名 → Tauri 名）：

| Electron | Tauri | Electron | Tauri |
|---|---|---|---|
| `window:minimise` | `window_minimise` | `window:close` | `window_close` |
| `window:toggle-maximise` | `window_toggle_maximise` | `window:unread-badge` | `set_unread_badge` |
| `window:is-maximised` | `window_is_maximised` | `prefs:save` | `prefs_save` |
| `prefs:theme` | `sync_theme` | `shell:open-external` | `open_external` |
| `dialog:pick-folder` | `pick_folder` | `shell:open-file` | `open_file` |
| `dialog:save-text` | `save_text` | `shell:open-workspace` | `open_workspace` |
| `dialog:save-bytes` | `save_bytes` | `shell:reveal` | `reveal_path` |
| `shell:reveal-logs` | `reveal_logs` | `shell:reveal-workspace` | `reveal_workspace` |

波次 6 新增的 15 条命令：

| 类 | Tauri 命令 |
|---|---|
| 系统能力（6） | `window_capture`、`shell_graphics`、`shell_global_shortcut`、`shell_set_global_shortcut`、`shell_process_memory`、`text_context_menu` |
| 浏览器子 WebView（9） | `browser_show`、`browser_hide`、`browser_freeze`、`browser_control`、`browser_navigate`、`browser_trust_certificate`、`browser_answer_login`、`browser_pick`、`browser_pick_cancel` |

事件订阅（不计入 `NAMES`，经 `plugin:event` 转发）：`onDropPaths`（`reasonix://drag-*` /
`tauri://drag-*`）、`onBrowserLoadState`（`browser://load-state`）、`onBrowserLogin`
（`browser://login`）。

残差三处，都是设计差异而非漏改：

| 项 | 性质 |
|---|---|
| `pathForFile` | 刻意移除。同步逐文件取路径被原生拖放路径事件 `onDropPaths` 取代；`ShellBridge` 仍留可选声明，`NativeHost.pathsForFiles` 以 `?.() ?? ""` 降级 |
| `prefs:load` | 设计差异：prefs 在启动时注入脚本，不走命令 |
| `browser:verify` | Electron 的 `verifyBrowserElement` 独立通道在 Vue 端口没有声明；re-find 逻辑（`pick.rs::lands_script`/`lands_match`）折进 `browser_pick` 流程内部使用（`views.rs`），不单独暴露 |

为什么该行仍是 `partial`：契约层 33/34 已注入且守卫绿，但原生子 WebView 的隔离、
拖放合成、登录/证书/挑选只在单测边界证明，未在真实 WebView2 窗口跑过（B1）；
macOS/Linux 的子 WebView 与拖放未在本机（Windows）运行。无平台等价物的动词显式回答
unsupported，不伪造成功。

## Electron 退役计划（未完成，仅原则）

D4 的裁决：不许在对等之前删除，不许被 Tauri 构建加载。

当前状态：`desktop/` 完全未改，`Makefile` 的 `studio` 目标仍指向
`desktop/electron/run-studio.sh`。

退役顺序尚未写进任务分解，属于本计划的未完成部分。已知前提：先把上表的 `partial`
变成 `done`。波次 6 已补齐 bridge 方法（33/34），剩下的 `partial` 主要是真实窗口验证
（B1）与 94 条文件级可达性债务（34 冗余内联 + 19 测试夹具 + 41 子系统阻塞，见 B9–B14）。
