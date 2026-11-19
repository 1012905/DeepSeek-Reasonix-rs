---
owner: "@SivanCola"
backup: "@esengine"
status: active
reviewed: 2026-11-17
---

# 迁移裁决

合并三棵树过程中的裁决。每条写「问题 / 裁决 / 依据」。

## D1 —— 主干：唯一写入目标

**问题**：三棵树各有内容，哪一个承载合并后的项目。

**裁决**：`C:\Users\10129\Desktop\Projects\DeepSeek-Reasonix-rs` 是主干，是合并工作
唯一的写入目标。

**依据**（2026-11-16 实测）：

| 树 | Go 内核 | Electron React | Tauri + Vue | 独有价值 |
|---|---|---|---|---|
| 主干 | 完整，`go.mod` 69 依赖 | 完整，1141 个 `.tsx`（非测试 438） | 317 `.vue`、768 `.ts`、284 测试文件、2140 测试绿 | 唯一有可用 Vue 壳的树 |
| A | 完整 | 完整 | `src/` 只有 5 个文件 | 5 个主干没有的文件（见 D2） |
| B | 无 | 无 | 无 | 25 个 `.rs` / 1585 行，M0 骨架 |

```
cd desktop-tauri && ./node_modules/.bin/vue-tsc -b --force   # exit 0
cd desktop-tauri && ./node_modules/.bin/vitest run            # 284 files, 2140 tests passed
```

A 是同一仓库的 Gitee 快照，tip 为 `3b0695c28`，建立在 `f2c82c2b6` 之上；而
`f2c82c2b6` 是主干 HEAD 的祖先，主干在那之后有 15 个提交。

实测 diff 见 `migration-state.md`：仅 A 有 5 个文件，仅主干 1281 个，内容不同 71 个。

B 只有一条提交 `87bfa14`（M0 骨架），ledger 从 `CLI-004` 起是 `todo`。

## D2 —— A 如何吸收

**问题**：A 有主干没有的东西吗，要不要复制回来。

**裁决**：A 只读。两个文件是摘取候选并已评审，其余不复制。

| A 独有文件 | 处置 |
|---|---|
| `desktop/frontend-next/src/ui/SidebarNav.tsx` | 只作 Vue `Sidebar` 的参考，不复制 |
| `desktop-tauri/src/port/wire.parity.test.ts` | 只看其线缆对等思路，对着新面重写属后续任务（B4） |
| `desktop-tauri/src/App.vue` | 已被 `ui/App.vue` 取代 |
| `desktop-tauri/src/stores/kernel.ts` | 已被 `state/*` 取代 |
| `.github/workflows/v1-maintenance-schedule.yml` | v1 线已退役，不吸收 |

**依据**：这就是完整的 A 独有集合 —— 实测而非估计，`diff -rq` 正好报 5 条，
`stores/` 一行展开为 `stores/kernel.ts` 一个文件。

A 的 `desktop-tauri` 是主干自身工作的更早状态：其 `src` 只有 5 个文件，主干同名目录
有 1210 个。覆盖即回归，所以不做任何目录级复制。

## D3 —— B 如何吸收

> **已被 D14 取代（2026-11-18）**：用户裁决采纳 B 上游最新版（Gitee `9b3d7ba`，M2）
> 的内核重写，作为 `RUST_MIGRATION.md` 的唯一 Rust 内核实现。下文为原始裁决，仅存档。

**问题**：B 把 Go 内核重写成 Rust，这与本次迁移的后端决策相反。

**裁决**：B 的方向**不在本次迁移范围内**，其 crates 不合入。

**依据**：迁移目标写的是「保留 Go 服务/模块；Rust 仅用于 Tauri 命令、插件、系统能力
桥接、性能敏感逻辑。禁止无必要重写 Go」。B 做的正相反 —— 它启动了一次完整的
Go→Rust 内核移植。合入会把两个内核放进一个仓库。

B 的贡献与保留方式：

| 资产 | 处置 |
|---|---|
| `docs/adr/0001-crate-layout.md` | **计划**归档到 `docs/archive/studio-rs/`；该目录尚未创建，见 B6 |
| `docs/parity/LEDGER.md` | 其**格式**被本迁移的完成度表采用；其**内容**（19 行：11 done / 1 wip / 6 todo）待归档，同属 B6 |
| 其余 Rust crates | 留在 B，不动。重启条件见 B3 |

**一处未解决的冲突**：`docs/DOCS_STANDARD.md` 的 C2 规则写「Git history is the
archive. Do not add `archive/` directories」，与本条要建的归档目录直接矛盾。
二者须择一，见 `blockers.md` B6。尚未裁决。

## D4 —— 桌面壳：Tauri 2，不是 Electron

**问题**：哪个是产品壳。

**裁决**：`desktop-tauri` 是产品壳。Electron 作为可用参考留在树里，直到 Tauri 壳达到
对等后才退役 —— 对等之前绝不删除，也绝不被 Tauri 构建加载。

**依据**：`src-tauri/tauri.conf.json` 已声明 Tauri 2
（`"$schema": "https://schema.tauri.app/config/2"`，`@tauri-apps/api ^2.9`）。

## D5 —— 前端：Vue 3 组合式 API，不上 router，不上 Pinia

**问题**：目标里点名了「状态/路由按原项目实际选型映射」，所以要按原项目实际用的
东西来，而不是按流行的库来。

**裁决**：保持移植的既有形状。

| 项 | 裁决 | 依据 |
|---|---|---|
| 组件 | `defineComponent` / `<script setup>` + 组合式 API | 与既有 317 个 `.vue` 一致 |
| 路由 | 不上 `vue-router` | React 的「路由」在 `App.tsx` 里是 UI 状态（`settingsOpen`、pane view、会话选择），已在 Vue 侧保持为响应式状态 |
| 状态 | 不上 Pinia | 共享状态住在 `src/state/*` 以模块级响应式导出，是 React 模块级 store（`useFolded`、`usePrefs`）的移植 |

引入 router 会给「当前画哪一屏」造出第二个真相源。Pinia 会与现有 `state/` 模块重复，
而原项目用的是模块 hook，不是 store 库。

**一处诚实脚注（实测）**：`pinia ^3.0.0` 仍列在 `desktop-tauri/package.json` 的
dependencies 里，但 `grep -rn pinia src/` 返回空 —— 没有 import、没有 `defineStore`、
没有调用点。它是死重量，不是第二层状态。删它本是一行清理，但实测被 lock 脱钩挡住
（`pnpm-lock.yaml` 仍停在 4 依赖时代，重对齐会剪 naive-ui、bump 补丁版，远超删 pinia），
已搁置并记为独立待评审缺陷，见 `blockers.md` B16。

## D6 —— 通信：kernel HTTP/SSE，Tauri 命令只管壳能力

**问题**：webview 与内核之间走什么，哪些能力归原生壳。

**裁决**：webview 通过 HTTP + SSE 与 Go 内核通信，与 React 应用完全一样（`SseHub` 在
`port/hub.ts`）。Tauri `invoke` 只保留给只有原生壳才做的事：窗口控制、外部打开/揭示、
文件夹选择、保存对话框、prefs 文件、主题同步、角标。`commands.rs` 声明该清单
（`NAMES`）。没有业务逻辑搬进 Rust。

**依据**（波次 6 实测覆盖度）：

| 口径 | 数量 |
|---|---|
| `commands.rs::NAMES` 声明的动词 | 31 |
| `bridge.rs` 注入 `window.reasonixHost` 的方法 | 33 |
| `port/host.ts` 的 `ShellBridge` 声明的方法 | 34 |
| 无 Tauri 实现的 bridge 方法 | 1（`pathForFile`，刻意移除，见 D12） |

31 命令 + 3 个 event 订阅（`onDropPaths`/`onBrowserLoadState`/`onBrowserLogin`）= 34
减去 `pathForFile`。波次 5 缺的 17 个里，6 个系统能力与 9 个浏览器动词已补；
`prefs:load` 仍在启动时注入 bridge 脚本而非 IPC 读回；`browser:verify` 折进 pick（D12）。

Vue 侧把这些当可选成员调用并带降级路径，所以残差是静默的而不是致命的。
真实窗口验证受 B1 阻塞，`migration-plan.md` 对应那一行保持 `partial`。

## D7 —— 并发：并行子 Agent，文件所有权不重叠

**问题**：剩余工作能否并行，靠什么保证不互相踩。

**裁决**：波次并行执行。约束是**子 Agent 只拥有它创建的文件**。

主 Agent 所有（接线在波次之间发生）：`App.vue`、`Pane.vue`、`main.ts`、
`src-tauri/**`、`port/**`、`i18n/**`。需要改这些文件的子 Agent 报告所需契约，
而不是直接编辑。两个任务需要同一文件时，放在不同波次。

**依据**：迁移目标要求并行（「并行是默认 … 上不封顶」），覆盖了
`Downloads/DeepSeek-Reasonix-studio-RS/AGENTS.md` 里要求串行的说明。

**对波次 1 记录的更正**：波次 1 曾断定本机**完全没有**子 Agent（每次派发都被拒，
错误为 `Agent type 'general-purpose' is disabled via disallowedTools
configuration`）。

该结论已不成立 —— 当前主机支持最多 **8** 路并行子 Agent，波次 2–5 确实用了它们。

仍不稳定的是子 Agent 后端的 DNS（`chickener.top`），多次解析失败；派发报错时，
文件写入可能已经落盘。所以这五份文档里的每个数字都是重新跑命令得到的，
不取自子 Agent 自己的报告。见 `blockers.md` B7。

## D8 —— 设置中心契约：单个 `settingsProps` prop，registry 用 glob

**问题**：设置面板是 20 个 section 页，每页都要十几个端口：window hub、pane 的
agent port、network 作用域 port、workspace 作用域 port，加 theme / contrast / weight
状态、status，和改变它们的回调。有两条路：

- (a) 每个 section 通过 `inject` 去取自己要的，就像 `SettingsLeave` 那样。
- (b) 每个 section 接受同一个 prop 对象并从中解构。

(a) 的失败模式到运行时才可见：没有祖先提供的 injected key 解出 `undefined`，
Vue 不报错，页面渲染成半空白 —— 正是本迁移已经吃过的那一类失败
（`SettingsPageStyle` 的 `foreign` 规则，C4）。它还让契约变成每 section 一份，
于是 20 个文件各自为同一个端口发明拼法。

**裁决**：

- `ui/settings/sections/` 下每个文件都声明
  `const props = defineProps<{ settingsProps: SettingsProps }>()` —— 只有一个 prop。
- `Settings.vue` 以 `<component :is="view" :settings-props="props" />` 渲染它。
- registry 由 `import.meta.glob("./sections/*.vue", { eager: true, import:
  "default" })` 构建，按文件名做键：`common.vue` 应答 `common` section。
- 没有对应文件的 section 在 `SECTION_VIEW` 里**故意缺席**，面板把它画成「尚未移植」，
  而不是去猜一个相邻页面。

**依据**：

| 证据 | 内容 |
|---|---|
| `ui/settings/registry.ts` | `VIEWS` 来自 `import.meta.glob`；`SECTION_VIEW` 按文件名做键；`sectionView()` 回落到 `pageHome(section)` |
| `ui/Settings.vue:257` | `<component :is="view" v-if="view" :settings-props="props" />` |
| 20 个 section 文件 | 逐个 `grep -c settingsProps` 均 ≥ 1 |
| 集合比对 | `sections/*.vue` 名字集合 == `prefsnav.ts` 的 20 个 `Section` 成员，实测 |
| `ui/Settings.test.ts` | mock 组件声明 `props: { settingsProps: { type: Object, required: true } }`，并断言 `hands each section the sheet's own props as one settingsProps object` |

**为什么一个 prop 而不是 20 个 prop**：类型化对象上缺一个成员，是**读它的那个
section** 的编译错误；而一个 section 从没要过的 prop 缺失，是一张渲染为空的页面。
类型是 `SettingsProps`（`ui/SettingsProps.ts`），面板只需满足它一次，每个 section 都
继承这个保证。

## D9 —— 折叠子页用 provide/inject 去重，按列作用域

**问题**：八个 section 有两种拼法。`FOLDS_INTO` 把 `memory` 与 `usage` 折进
`storage`，`versions` 折进 `account`，`remote` 折进 `network`，`persona` 与 `peers`
折进 `session`，`providers` 折进 `model`，`hooks` 折进 `ext`。

每一个也都需要自己的文件，因为一个链接 —— 或模型点名的一个设置 —— 会直接把面板开到
`memory:memory` 或 `versions:logs`。于是同一个块能通过两条路到达，朴素的移植会在一列
里画它两次：两个 `set-<anchor>` 元素让落点标记与搜索命中变成掷硬币，读者没被送去的
那一份，在另一份从脚下滑走时永远够不到。

**裁决**：页面在画任何东西**之前**声明自己是哪个 section（`declarePage(page)` 放在
`setup` 顶部）；section 去问上面那页是不是正是自己（`drawsHere(own)`），是则渲染为空。

声明通过 `provide` / `inject` 沿列向下传，键是 `Symbol`，从不是模块级变量：

| 性质 | 为什么 |
|---|---|
| 按列而非全局 | 两个开着面板的窗口读不到彼此的答案 |
| 导航时重来 | `Settings.vue` 把 `.prefs-col` 以 section 为键，替换该列就是导航本身 |
| 外面没有面板也能用 | 单独挂载的 section 也是页面，`inject` 解出 `null`，于是它画自己 |

**依据**：`ui/settings/sections/nesting.ts` —— `PAGE` 是
`InjectionKey<Section> = Symbol("settings.section.page")`；`declarePage` 调
`provide`；`drawsHere` 调 `inject`，**并且**重新 `provide` `own`，好让它嵌套的东西
也能知道上面已有一个同名页面。

实测调用点：

| 函数 | 文件 |
|---|---|
| `declarePage` | `account.vue`、`network.vue`、`session.vue`、`storage.vue` |
| `drawsHere` | `memory.vue`、`peers.vue`、`persona.vue`、`remote.vue`、`usage.vue`、`versions.vue` |

剩下两个 `FOLDS_INTO` 项走另一条路，不需要守卫：

- `providers.vue` 渲染的是 model 页渲染的同一个 `CoreProviders` part —— 一份实现、
  两个入口、没有重复锚点。
- `hooks.vue` 只被画一次，由 `session.vue` 传 `:settings-props="settingsProps"` 渲染。
  这跟随 React 基准：那里 `Hooks` 是在 `AgentSettings`（session 页）里渲染的，
  不在 extensions 页里。只有一个宿主，没有要去重的东西。

**为什么不用 prop、store 或模块级标志**：prop 得逐层穿过每一跳嵌套，还会让 section
自身的正确性取决于它的调用者。store 或模块级标志是全局的，会为错误的列作答。
`provide` / `inject` 是本树里 `SettingsLeave` 已经确立的形状，一种惯用法而非两种。

## D10 —— 扩展名纪律：只有命名导出的 TS 模块不是 `.vue`

**问题**：五个文件带着 `.vue` 扩展名，内容却是裸 TypeScript —— 命名导出，没有
`<script>` 块，没有 `<template>` 块：`ui/SettingsLeave`、`ui/panels/kit`、
`ui/chart/plots`、`ui/GitPushCard`、`ui/RuntimeReload`。

`vue-tsc` 满意，`vitest` 也满意（它把它们当模块 import）。`vite build` 不满意：
`.vue` 会把文件送进 SFC 解析器，而它要求至少有 `<script>` 或 `<template>`。

用 `vue/compiler-sfc` 自己的 `parse` 实测：裸 TS 文本当作 `.vue` →
`At least one <template> or <script> is required in a single file component.`
而只有 `<script>` 没有 `<template>` 的文件能正常编译。

所以缺的是**块**，不是 template —— 这个区别重要，因为 `ui/vendormarks.vue` 与
`ui/SettingsPageStyle.vue` 同样没有 `<template>`，却完全合法。

typecheck 绿 + 测试绿 + build 红是最糟的组合，因为两个便宜的闸门都说活干完了。

**裁决**：扩展名选择编译器，所以它必须与文件是什么相符。

| 文件是什么 | 扩展名 |
|---|---|
| 只有命名导出的模块 | `.ts` |
| 带 `<script>` 或 `<template>` 块并导出组件的 SFC | `.vue` |

默认导出与命名导出都算组件（`ui/vendormarks.vue` 以命名方式导出 `VendorMark`，
合法）。改名是修法，不是风格偏好。

**依据**：这五个现在都以 `.ts` 存在（`SettingsLeave.ts`、`panels/kit.ts`、
`chart/plots.ts`、`GitPushCard.ts`、`RuntimeReload.ts`），没有 `.vue` 孪生文件；
`vite build` 退出 0（3208 模块）。

它们的 importer 用无扩展名路径（`Settings.vue:13` import `./SettingsLeave`；
`panels/Agents.vue:18` import `./kit`；`workbench/WorkbenchPanel.vue:35` import
`../GitPushCard`），所以改名除了那些显式写了 `.vue` 的路径，没有引起调用点改动。

## D11 —— 守卫携带记录在案的债务，一文件一行，债务缩小就失败

**问题**：本树里两个守卫本身成了它们要防的失败模式。

- `styles/mono.test.ts` 断言的 `MIXED` 集合是 React 基准的不完整子集。
- `orphan.test.ts` 更糟：它的 glob 要 `.tsx`，而 Vue 树里有 **0** 个，于是它什么也
  没读、什么也没断言，却对着 321 个它从未看过的组件报绿。

一个无法失败的守卫，与一个通过的守卫无法区分；它比没有守卫更糟，因为会被当作证据
引用。

**裁决**：针对未完成工作的守卫，把债**显式写在文件内，一文件一条并附理由**，
并且有三个测试：

1. 不可达的东西不得**未登记** —— 新的孤儿会失败。
2. 登记的东西必须**仍然是**孤儿 —— 债务缩小时必须删掉该条。
3. 守卫必须证明它**读了**它声称在守的树。

债务按文件登记，绝不按计数：计数会让「接通一个文件的同时孤立另一个」相互抵消，
悄无声息。

**依据**：

| 证据 | 内容 |
|---|---|
| `KNOWN_ORPHANS` | 129 条的 `Map<string, string>`（文件 → 理由） |
| `ENTRIES` | 声明为 `["src/main.ts", "src/testsetup.ts"]`，不靠文件名推断 |
| 遍历的四种边 | 字面 import / export、`import()` 分块、worker 的 `new URL(..., import.meta.url)`、以及 `import.meta.glob` |
| 三条测试 | `does not leave a file nothing imports`、`keeps every recorded orphan an orphan`、`reads the tree it is meant to be guarding` |
| 实测 | `vitest run src/orphan.test.ts` → 3 passed |
| `mono.test.ts` | 同形：`MIXED` 19 条，第三条测试 `keeps every recorded exception in use`，3/3 通过 |

`import.meta.glob` 这一条是这个守卫的关键：它是读 import 语句的工具看不见的边，
而 20 个设置 section 恰好通过一个 glob 挂载 —— 漏了它，每个 section 页都会被读成孤儿。

**这件事实话，不修**：它**不**为了把测试变绿而扩大集合。

`mono.test.ts` 的 `MIXED` 保持 19，而 React 基准有 27。多出的 8 个基准类
（`cost`、`cur`、`delta`、`meta`、`path`、`pt`、`rmtcand`、`uspan`）是**记录在案的
已知差异**，不是被填平的洞 —— 见 `blockers.md` C6。

为落地一次改动而扩大它，正是那个文件注释所禁止的动作。

## D12 —— 拖放走原生路径事件；子 WebView 隔离；无等价物显式 unsupported

**问题**：波次 6 补齐壳能力时，三处需要裁决而不是默认照搬 Electron。

**裁决**：

1. **同步 `pathForFile` 退役，改原生拖放路径事件。** Electron 逐文件同步问路径
   （`pathForFile(file)`）在 Tauri 没有等价物。改为壳的原生 drop 事件携带路径列表
   （`onDropPaths`），`dropwin.rs` 在 Windows 发 `tauri://drag-*` / `reasonix://drag-*`，
   `bridge.rs` 合成 DOM `DragEvent` 并把路径交给 `filedrop.ts` 的 router。
   `ShellBridge.pathForFile` 保留为可选声明，`NativeHost.pathsForFiles` 以 `?.() ?? ""`
   降级；浏览器标签页本来就拿不到路径，答 `[]`。同步取路径不再是契约的一部分。

2. **`browser:verify` 折进 pick，不单独暴露。** Electron 的 `verifyBrowserElement`
   独立通道在 Vue 端口没有声明。re-find 一个 selector 是否仍命名同 tag 元素的逻辑
   （`pick.rs::lands_script` / `lands_match`）在 `browser_pick` 流程内部使用
   （`views.rs`），作为「答案是否可信」的判据，而不是给页面的独立动词。

3. **子 WebView 与主 webview 隔离，能力只紧不松。** 每个 target 独立持久化存储
   （`data_directory` 按 partition）；内核 origin 永不可作为子页面导航目标；凭据 cookie
   只设在主 webview，子 WebView 拿不到内核凭据也拿不到壳桥接（capability 绑定
   `.window(MAIN).webview(MAIN)`，初始化脚本只注入主 webview）；弹窗 Deny、下载 deny、
   地理/通知/摄像头/麦克风/剪贴板权限拒绝。登录与证书信任须用户确认。

4. **无平台等价物显式回答 unsupported，不伪造成功。** `additional_browser_args`
   在 macOS/Linux 是 no-op；`GraphicsState.launched_off = off && cfg!(windows)`；
   读不到 GPU 状态答 unknown；`cdp`/`certs` 在非 Windows 是诚实的 unsupported 桩。
   软件渲染设置真正写进 Windows 启动参数（`--disable-gpu`）。

**依据**：`bridge.rs` 精确名单守卫与 `commands.rs` ACL 守卫绿（31 命令）；
`policy.rs`/`pick.rs`/`protocol.rs` 单测覆盖导航策略、答案 allow-list、re-find；
`cargo test` 90 绿。**未验证**：真实 WebView2 窗口里的隔离与拖放合成（B1），
macOS/Linux 子 WebView 与拖放未在本机运行 —— 这些标 partial，不声称已验证。

## D13 —— 功能已内联的冗余组件：保留孤儿记录，不为消债强换

**问题**：波次 7 孤儿接线区分两类孤儿。一类是「机件齐全、live 汇合点忘了 import」
（RuntimeReload / RemoteNotice+stale_page / links），接线即消债，已做。另一类是
「React 用独立组件，Vue 把同一功能内联进了 live 面板」——组件本身因此成为孤儿，
但功能并未缺失。`MemoryProactiveRow.vue` 是实例：React `Memory.tsx:188` 在 `.mem`
顶部挂 `<MemoryProactiveRow port>`（`.memproactive` 样式，suggest.css）；Vue
`settings/parts/data-memory.vue` 已把同一控制内联为 `.lrow` + `.seg` radiogroup
（:79-80/104/170-173/193-204，自带 PROACTIVE 常量、memoryProactive/setMemoryProactive
调用、错误回落），live 且有测试覆盖，样式与面板其余行一致。

**裁决**：这类「功能已内联」的冗余组件**保留 KNOWN_ORPHANS 记录，不为消债强换**。
理由：(1) 功能不缺，验收基线（原项目功能）已满足；(2) 独立组件与内联版 markup/样式
不同（`.memproactive` vs `.lrow/.seg`），强换会改视觉节奏并可能破坏面板既有邻近测试
——「破坏邻近行为的修复是失败的修复」；(3) 把working 的内联实现换成另一样式组件属
与迁移合并无关的重构，指令禁止。消债不是目标本身，行为对等才是。

**适用边界**：仅当内联版功能对等、live、有测试时适用。若内联版缺功能或无测试，
仍按基准接独立组件。每条此类孤儿在 migration-state.md 注明「功能已内联于 X」，
使「97 条债务」不被误读为「97 个缺失功能」——其中一部分是冗余而非缺口。

**依据**：data-memory.vue 内联 proactive 控制实测 live；MemoryProactiveRow.vue
类型检查通过但无 live 消费点；二者调用同一 port.memoryProactive/setMemoryProactive
（MemoryHoldPort，mock_memory_hold.ts:7 / sse.ts:109 实现）。

## D14 —— 采纳 B 上游最新版的内核重写（取代 D3）

**问题**：D3 拒绝合入 B 的 crates，理由是「避免一个仓库放两个内核」。但主干自身的
`docs/RUST_MIGRATION.md`（reviewed 2026-11-13）早已把 Go→Rust 内核重写列为既定方向，
A 谱系的 `crates/` 就是它的起步（10 个 crate 里 9 个是 1 行 stub，仅 `contract` 有
2016 行实代码）。所以「两个内核」的张力其实是 A 谱系与 B 谱系之争，而非「要不要 Rust
内核」。用户裁决（2026-11-18）：**采纳 B 的内核重写方向，但用上游最新版**
（Gitee `rikkarikka/reasonix-studio-rs` main，HEAD `9b3d7ba`，M2），不是本地陈旧 B
（`87bfa14`，M0，crates 仅 348 行）。

**裁决**：以上游 B（M2，87 个 `.rs`／33 751 行，账本 563 行＝69 done／1 wip／493 todo）
作为 `RUST_MIGRATION.md` 的**唯一 Rust 内核实现**，取代 A 谱系的停放骨架。Go 内核、
Tauri 壳、Vue 前端**保持 live**（RUST_MIGRATION 决策 5「两栈共存至 parity 签收」）。
**不做 cutover**：`apps/reasonix-studio-host` 仍是 stub（每次调用 exit 2，HOST 行在 M4），
Rust 内核远未对等，Go host 仍是实际后端。

**合并裁决（按模块/文件择优并留痕，非整目录覆盖）**：

| 资产 | 处置 | 理由 |
|---|---|---|
| A 的 9 个 stub crate（base/boot/model/runtime/safety/serve/session/state/tools） | 由 B 的真实 crate 取代 | B 同方向且远更成熟；A 仅 1 行 doc |
| A `contract` 的 eventwire（+golden manifest+parity 测试）/pricing/agentgraph/delivery/surfacespec | **保留**，并入 B 的 `reasonix-contract` | B 账本 SSE-004 eventwire 仍 todo，A 的 golden 门领先；两 crate 模块名零冲突，A 模块仅用 serde/std、无 unsafe、`crate::` 只引这 5 个模块 |
| B 的 apps/xtask/Cargo.toml/Cargo.lock/clippy.toml/deny.toml/rustfmt.toml/rust-toolchain.toml/.cargo | 采纳 | 内核工作区本体 |
| 根 `Cargo.toml` | 采纳 B 的，**加 `exclude = ["desktop-tauri/src-tauri"]`** | 壳保持独立工作区，其依赖图不进内核 Cargo.lock |
| B 的 docs/contracts、docs/parity/LEDGER.md、docs/adr/0001 | 采纳为 trunk **live** 文档 | 冻结契约＋验收账本＋crate 布局裁决 |
| B 的 AGENTS.md / README.md | 存为 `docs/kernel-rs/`（provenance） | trunk 根 AGENTS.md/README.md/REASONIX.md 是 Go 内核 standing instructions，**不覆盖** |
| 根 rust-version | 1.85 → 1.99.1（B 的 pin，本机已装） | 壳 rust-version=1.90 ≤ 1.99.1，edition 2024 兼容 |

**B6 冲突就此消解**：B 的 ADR/ledger 既被**采纳为 live**，就不再需要 `docs/archive/studio-rs/`
归档目录，`DOCS_STANDARD.md` C2「Git history is the archive，不加 archive/ 目录」得以遵守。

**验证（本机实测，USTC 镜像）**：`cargo test --workspace --all-features` → **572 passed／0 failed**
（含 A 保留的 `eventwire_parity` golden 套件）；`cargo fmt --all --check` 0；
`cargo clippy --workspace --all-targets --all-features -- -D warnings` 0（零 warning）；
`cargo xtask parity --check` OK（563 行）；`cargo xtask deps --check` OK（19 manifests／6 internal edges）；
`go build ./...` 0 且 `internal/`、`cmd/`、`go.mod` 零改动。`cargo-deny/nextest/machete`
本机未装，记 skipped（见 B15）。

**网络**：crates.io 与 rsproxy CDN 本机 DNS 间歇失败；按用户「网络原因请走镜像」在
`~/.cargo/config.toml` 配 USTC sparse 镜像（机器级，**不提交进仓库**）；仓库 `.cargo/config.toml`
只保留 xtask alias。

**边界**：内核未对等（493 todo），cutover 阻塞见 B15；上游 clone 留在
`Downloads/reasonix-studio-rs-main` 作来源记录，不动原项目与本地 A/B。

## D15 —— 波次 11 前端收口：可移植的移植，结构性不可达的有界裁决

用户指令「全部继续，无需更改的 Go 代码不动，仅维持部分重构 Rust+Go 局面」「继续，一次完成」。
本波次 Go/Rust 内核**零改动**（不推进 cutover，B15），仅 Vue 前端把剩余子系统驱动到终态：
能忠实移植的整条移植并接线，结构性不可达或依赖未移植子系统的余项**有界裁决**而非半措施。

**已移植并接线（done，主 Agent 重跑验证）**：

| 簇 | 落地 | 消债 |
|---|---|---|
| B12-core pane-opening | `App.vue` 每-pane `viewRequests` Record + `openPane` + `requestView/showView`，供 Pane 与 `useBranchSession` | 解 B14 branch 前置 |
| B11-Files rail | `Metrics.vue` ORDER 增 `files` 并挂 `Files`；`Pane.vue` 派生 `railOf` 喂 changes/tree/yolo | panels/Files、panels/derive 转 reached |
| B14 运行时动作 | 每会话 live 状态（B12 的 runs Record）门控 pause/close/branch/archive-live；Sidebar 接 on-pause/on-close-conversation/on-branch | — |
| B10b agent-runs 工作台 | `useAgentRequest`+`AgentRunView`+`useAgentTabs` 进 WorkbenchPanel；Pane provide AgentRunOpener、reveal=openColumn("files") | useAgentTabs 转 reached（84→83） |

**有界裁决（终态，保留孤儿记录，非半措施）**：

- **B13 工作台浏览器标签改名 → 移出 Vue 范围**。浏览器在 Vue 走停靠浏览器列（D6/D12），
  不进工作台标签条；改名 browser-only，故 `useTabNames` 结构性不可达，接上是死代码。
  按 D13 保留孤儿。这是架构分歧的终态裁决，不是待补缺口。
- **B12 sidebar-open-to-flow → 内联（D13）**。`App.openSession` 重点击已开会话即
  `requestView(id,"flow")`，正是 useSidebarOpen 语义；`sidebaropen.ts` 功能已内联，保留孤儿。
- **B12 并发开 ticket 排序（useSessionSwitch）→ 暂缓**。Vue 用串行 busy 守卫 openPane，
  无并发 ticket 消费点；`sessionswitch.ts` 保留孤儿，待 Vue 引入并发开 pane 才有意义。
- **B10c job 查看器（JobRunView）→ 移出当前范围**。`openJob` 已落地但 Vue 无 job-opening
  调用点（无后台 job Surface 来源）；runs 是 live 路径，job 查看器待调用点出现再移植。
- **B10d 「回到分组」onShowCall → 暂缓**。依赖 Transcript focus-scroll 子系统（Vue `focus`
  prop inert）。`AgentRunView` 以 onShowCall 可选门控，缺则**不画**返回按钮（诚实缺席，
  非 inert 按钮），Pane 暂不供给。待 focus-scroll 移植再接，避免无机制支撑的死按钮。
- **B14 kept-versions / onMoveTo → 暂缓**。分别依赖 versions+copies 分散 UI、
  `useSessionMoveTo`＋项目选择对话框，Vue 均未移植；组件以可选回调门控（缺则不画）。
- **B11 Plan 面板 / conversation-files 链 → 暂缓**。Plan（React ui/Plan.tsx）与
  useConversationFiles（filelinks/grantedfiles/WorkbenchFileMenu）整条链未移植。

**为什么这层**：每项裁决都落在 owning 层并给出精确架构原因（结构性不可达 / 依赖未移植
子系统 / 功能已内联），不以「接上再说」制造死代码或 inert 控件。可移植的整条移植（B10b/
B11-Files/B12-core/B14-runtime），不可移植的诚实缺席＋记录，符合「禁止半措施」「修复落在
owning 层」「A fix that breaks a neighbouring behaviour is a failed fix」。

**验证（本机实跑，主 Agent 重跑非子 Agent 自报）**：`vue-tsc -b --force` EXIT 0；
全套 `vitest run` **304 文件 / 2250 测试**全绿 EXIT 0；`vite build` EXIT 0（3292 模块）；
定向（WorkbenchPanel.interaction 6 + agent-run-tab 9 + useAgentRequest 8 + orphan 3 +
Transcript.subagents）5 文件/30 测试绿；orphan debt **83**。基线由波次 10 的 300/2216
增至 304/2250。Go/Rust 门本波次未触发（零改动）。

**边界**：D5（pinia 死依赖）仍 blocked B16（pnpm-lock↔package.json 脱钩需专门 resync，
不擅自升级依赖）；B15（Rust cutover）按用户指令不推进；真实窗口验证仍 B1。

## D16 —— 波次 12：D15 收口，前端 parity 清单归零（B13 除外）

用户指令：「1可以了，2提交，D5，D15，Electron 退役（需要你完成 Vue + TypeScript + Tauri 重构，
Rust 和 Go 共存）」。窗口由用户在本机终端验证可打开（B1 的用户侧解除）；本波次把 D15 的暂缓项
全部落地或保留为终态裁决，Go/Rust 内核零改动（维持共存，不推进 cutover）。

**落地（done）**：

| 项 | 内容 | 证据 |
|---|---|---|
| D5 | 删 pinia（lock −92 行，无 import） | 提交 `d3683628c`；全量绿 |
| B10c | Pane 提供 JobsLive、工作台 job 面渲染 JobRunView、工具卡 BackgroundNote 可达 | job tab + note 3 例 |
| B10d | Transcript 真正消费 focus（隐藏持有、折叠/切片/details 展开、按 n 一次消费）＋ Pane onShowCall | Transcript.focus 4 例 |
| B11 | Plan 面板（Metrics ORDER + Pane s.plan/blocked）；useConversationFiles（一次性 request/授权夹/GRANTED 图片/菜单/documentRefs） | Plan 6、convfiles 11、工作台链接 1 |
| B14 | kept versions（KeptRows + removeSession + 内联确认）；move-to（useSessionMoveTo + 对话框，持有中禁移） | keptversions 10、move-session 6 |
| B12 | useSessionSwitch 接入 openPane（ticket/flights/cancel/opening 反馈）；保留 root 的新会话；Sidebar 行标记与 aria-busy | sessionswitch 4 + App 契约 |

**保留的终态裁决**：

- B13 浏览器标签改名维持移出范围：Vue 是停靠浏览器列，browser-only 改名结构性不可达。
- `sidebaropen.ts` 维持 D13 冗余孤儿（语义已内联在 `openSession`）。
- `openCards.ts` 是另一条 card-opening 链，与运行面无关，保留孤儿。
- 动作名按 `actions.ts` census 校准：菜单 `session.move-to-start`、对话框提交 `session.move-to`。

**验证（主 Agent 重跑）**：

- `vue-tsc -b --force` EXIT 0；全量 `vitest run` **311 文件 / 2297 测试**全绿（基线 304/2250）。
- 定向 16 文件 / 145 测试；orphan debt **81**（83→81：jobcancel、sessionswitch 转 reached）。
- Go/Rust 门未触发（零改动）。

## D17 —— Electron 退役执行（P1）：删源、门禁跟随与两处 Go 工具的必要改动

用户裁决「全部退役」。执行结果：

- **删除**：`desktop/`（2423 个跟踪文件）。
  - 四条 Electron 工作流（`studio.yml`、`release-studio.yml`、`deploy-web-studio.yml`、
    `studio-selftest-windows.yml`）＋ Certum 签名冒烟、`.signpath/`、`cmd/signpath-contract/`。
  - `scripts/` 四个仅服务旧发布线的测试删除（`cli-archive-build`、`studio-release-flags`、
    `studio-artifacts`、`linux-install-smoke`）；另六个脚本裁掉"读已删工作流"的用例。
- **迁移**：
  - `desktop/cmd/studio-manifest` → `cmd/studio-manifest`（连同 `arch.json` 与其 gate 测试）。
  - `trace_readings.json` → `internal/frontend/serve/testdata/`（Go 测试从 git 历史恢复）。
- **门禁跟随（Go 工具必要改动，非内核行为）**：
  - wire-parity 的 TS 镜像常量改指 `desktop-tauri/src/port`；tag parity → `state/session.ts`。
  - route-parity 读 `desktop-tauri/vite.config.ts`；repolint `stylesDir`/size 豁免/layer host 随迁。
  - `baseline.json` 一次受控 `-update -allow-widen`（+141/−78：回收 desktop 条目、记录既有 docs
    债务与 Vue 样式表计数）。
  - Go 侧对等测试与 `serve/page.go` 开发回退路径重指 `desktop-tauri`（15 个文件，含注释镜像路径）。
- **配置/CI**：
  - `ci.yml` 去 desktop job/过滤/交叉腿与 gofmt 排除。
  - `dependabot` 换 `desktop-tauri`（npm）与 `desktop-tauri/src-tauri`（cargo）。
  - `labeler`/`CODEOWNERS`/`.gitattributes`/`.gitignore`（Wails 块、desktop env）清理。
  - `make studio` 已切 Tauri、`make test` 跑 Tauri 套件。
- **打包**：`bundle.active=true`（targets all、四套图标）。
  - 本机产出 MSI 4.44 MiB 与 NSIS 2.95 MiB（0.1.0 x64 en-US），未签名。
- **已知后续**：
  - CLI/brew 的发布步骤随旧工作流移除（打包脚本保留）；若继续发 CLI/brew，由 P3 补新工作流。
  - `backfill-studio-notes.sh --apply` 的 idle 轮询仍指向旧工作流，待 P3 落地后改名。
- **验证**：
  - `go build`/`go vet` 0；`golangci-lint v2.12.2`（GOTOOLCHAIN=go1.26.7）0 issues。
  - repolint 3556 baselined clean；`make check` 0。
  - `go test ./... -p 4 -timeout 20m`：211 包 ok；5 个路径遗漏失败重指后 10 个受影响包全绿。
  - sandbox/builtin 3 个失败在退役前标签同样复现（环境性，见 blockers.md），敏感路径未改。
  - 前端：vue-tsc 0；vitest 311/311 文件；vite build 成功。

**退役准备（先行步骤）**：oracle 对照冻结为 `src/ui/parity-contract.json` 与
`src/i18n/msg/oracle-named.json`（620 键）；Makefile `studio`/`test` 切 Tauri。
删除与发布去向的产品裁决见上文「全部退役」，执行顺序见 migration-plan.md 退役表。

## D18 —— P2：旧账号/云备份/市场发布界面退役（Go 云端半边剥离）

用户裁决「原来的账号部分一起去掉」。执行结果：

- **Vue 删除**：账号登录页与设置块、市场发布/投票/我的包（`ext-market.vue` 收敛为浏览+安装）、
  云备份整组、手机提醒/推送/云远程与 `RemoteAwareApp` 包装、反馈贡献榜/等级/账号关联
  （反馈对话框与提交/历史保留，通道本就 `FEEDBACK_RETIRED`）。
- **迁移**：TraceExport 迁至 storage 页（锚点 `storage:trace-export`，`settingScreens` 随指）；
  Versions 面板保留在设置目录（profile 等级展示随反馈等级退役）。
- **端口/线缆**：`AgentPort` 摘除 account/backup/market-publish/phone/cloud-remote/board 方法，
  sse/mock/boundary 同步；wire-parity 表摘除对应镜像对。
- **i18n**：按 msg.test 同口径批删孤儿键 **237** 个；`settings-intents` 去掉 account/backup 目标。
- **Go**：serve 删除 account/backup/cloud-presence/market-publish/phones 路由与文件（−2709 行），
  `startCloudRemote` 移除；DeviceShare 的 LAN/Tailscale 门完整保留（`test_share*`、tailnet 用例全绿）。
  删除随云端路由失去唯一调用者的 `notify.Phone`/`Sink.WithPhone`（孤儿门禁）。
- **保留**：`internal/platform/account`、`remotecloud` 休眠（CLI 登录不动）。

**验证**：

- `vue-tsc` 0；全量 vitest **297 文件 / 2170 测试**全绿（删测试即删用例）。
- `go build`/`go vet` 0；`golangci-lint` 0 issues；repolint 3553 clean。
- serve 全量 780s：唯一失败是 P3 的 notes URL 断言（已随 P3 更新并定向复跑通过）；
  cmd host、account、remotecloud、notify 全绿。

## D19 —— P3：更新通道改指本仓库 Releases

- **目录**：`update.StudioReleases` = `https://github.com/1012905/DeepSeek-Reasonix-rs/releases`
  （按本仓 remote 定），`StudioCatalog` = 其 `latest/download/versions.json`；notes 为各 tag 的
  `notes.md` 资产。重定向信任集不变（github.com/.githubusercontent.com）。
- **标识**：Mac BundleID 与 polkit action 改 `com.esengine.reasonix-studio`；instanceid 保持稳定
  （升级不搬状态目录）；`computer` 敏感路径未动（其 pin 属 CUA，另行处理）。
- **发布**：
  - 新 `release-studio-tauri.yml`：tag `studio-v*`，三平台矩阵。
  - Tauri bundle 携 `bundle.resources` 的 host `bin/` 与页面 `page/`。
  - minisign 旧密钥签名+公钥自验；studio-manifest 生成 latest.json，versions.json 挂 latest 资产。
  - candidate 不入目录。
  `withdraw-studio-version.yml` 从 R2 改为操作 latest release 的 versions.json。
- **脚本**：`update-versions-index.sh` 产 GitHub 资产 URL；`backfill-studio-notes.sh` 退役
  （notes 现为发布资产）；polkit 名称随标识更新。
- **验证**：update/instanceid/studio-manifest Go 测试绿；脚本测试 12+6 绿；actionlint 0；
  构建 side 待用户配置 secrets 后首个 `studio-v*` 触发实测。

## D20 —— P4：零安装网页访问（Worker + Go 中继 + 配对页）

用户要求「直接打开就能用」：浏览器无需安装，桌面无需公网地址。三路实现（Worker、
Go、Vue 端口/界面）＋主 Agent 接线裁决。契约落在 `workers/remote/README.md`：

- **Worker**：
  - `/v1/devices` 注册 X25519 公钥换 deviceId/credential；Bearer 换 6 位一次性码（300s）。
  - GET `/v1/pair/<code>` 换 controlTicket（600s）。
  - 桌面 WS `/v1/devices/<id>/connect` 保持 Go host 既有 gateway 信封（`relay_hello`、
    `controller_{connected,message,disconnected}`、`{"to","payload"}`）。
  - 浏览器 WS `/v1/sessions/connect` 用子协议 `reasonix.auth.<ticket>`（不能发 Authorization 头）。
  - Durable Object 每设备一房、休眠、转发密文；64KiB/1009，4004/4003/4401。
  - 新增 rotate/revoke REST；`bundle.resources` 让页面随 Worker 静态资产部署。
- **Go**：
  - `remotecloud.NewRelay` 用 Worker Registrar 替代 account.Client（保存设备身份）。
  - 凭证静态，不再轮询账号 token；401 停机而非静默重注册。
  - serve `/remote/relay*` 四端点按需 enable/disable/pair，host-only。
  - `REASONIX_RELAY_URL` 必填且为 wss。
- **Vue**：
  - 恢复 X25519/HKDF/AES-GCM 链路（`reasonix-remote-v1|deviceId`、session+seq 防重放、62KiB 帧预算）。
  - `relay_cloud.ts` 换票→WS→ready→SseHub；hosted 构建探测不到内核即渲染配对页。
  - 设置 → 远程新增「网页访问」卡（开关/6 位码）。
  - 重配对时换票必须绕开本模块已安装的隧道 fetch。

**验证**：Worker `tsc --noEmit` 0；Go build/vet/golangci 0，remotecloud/serve/cmd 全量绿
（serve 748s）；vue-tsc 0；vitest 303 文件全绿；vite build 成功；repolint clean；
actionlint 0；路由可达测试绿。浏览器↔桌面端到端与 Worker 部署需用户配置 secrets/KV。

## D22 —— 新 minisign 密钥替代旧密钥

用户裁决「重新生成」。原方案 D3/P3 复用退役发布线的旧密钥（Key ID `AF12CA46F4A9EBB0`），
改为生成一把新密钥（Key ID `441E92EFE08CA57F`），用于 Release + registry 签名。

- 私钥与口令由主 Agent 在仓库外生成，经 `gh secret set` 直接送入两个仓库的 Actions secrets。
- 全程不落盘仓库、不回显内容；0.1.0 尚未发布，无历史签名失效代价。
- 同步轮换：`verify.go` 公钥常量/注释、`release-studio-tauri.yml` 发布自验、`update_test.go`
  固定 ID、`STUDIO_RELEASE.md` 验签命令、registry 仓 `registry.pub`/`README.md`。

## D23 —— 发布标记按当前主版本行计算

旧 `studio-v2.x` 标签仍在仓库里，而新线从 `0.1.0` 起步。原 `studio-release-flags.sh` 把
所有稳定标签一起排序，2.61.0 永远最高，首个 0.1.0 将得不到 latest，目录无处可挂。
- 改为只比较**被发布版本的主版本行**（0.x 只看 0.x）。
- 首个 0.1.0 取 `--latest`；同一行内「恢复旧标签不夺回徽章」的语义保持不变。
- 历史 2.x 标签不再参与排序。

