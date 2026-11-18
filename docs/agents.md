---
owner: "@SivanCola"
backup: "@esengine"
status: active
reviewed: 2026-11-17
---

# Agent 编排

迁移怎么分、谁拥有哪些文件、每个子 Agent 允许动什么。最后更新：2026-11-17（波次 6）。

## 角色

- **主 Agent（编排 / 裁决 / 合并 / 验收）：** 拥有盘点、裁决、共享契约、文件
  所有权表、波次计划、跨 Agent 仲裁、波次之间的合并与验证、最终交付。不写业务
  模块代码；只写把子 Agent 产出拼起来的接线。
- **子 Agent（执行者）：** 各自一个可独立验收的范围 —— 一个模块、一组文件、或一个
  验证任务。读契约，只写自己的文件，绝不跨范围重构。

## 文件所有权

让并行安全的那条规则：**一个文件在同一时刻只有一个所有者。**

主 Agent 所有（子 Agent 不得编辑；需要改动时报告所需契约并等接线波次）：

```
desktop-tauri/src/App.vue
desktop-tauri/src/main.ts
desktop-tauri/src/ui/Pane.vue
desktop-tauri/src/ui/Metrics.vue
desktop-tauri/src/port/**
desktop-tauri/src/i18n/**
desktop-tauri/src-tauri/**
desktop-tauri/package.json
desktop-tauri/tsconfig.json
desktop-tauri/vite.config.ts
desktop-tauri/index.html
docs/**
```

`ui/Metrics.vue` 与 `docs/**` 是波次 5 新加进来的。前者因为它是面板的挂载点
（接线文件）；后者因为收口文档由主 Agent 一人写，避免五份文件被并行改写。

子 Agent 只在声明的目录里**创建新文件**。可以读主干里的任何东西和 React 基准。

所有权冲突靠「把任务移到后面的波次」解决，绝不靠两个 Agent 编一个文件。

## 波次与并行度

| 波次 | 并行度 | 任务 | 是否降级串行 |
|---|---|---|---|
| 1 盘点 + 裁决 | 3 路并发读，主 Agent 仲裁 | 主干、A、B | 否 |
| 2 缺失的屏幕 | 6 | settings、sidebar、chrome、onboarding、panels、browser | 否 |
| 3 接线 + 验证 | 1 路接线（主 Agent）后 3 路验证 | typecheck / vitest / build | 接线串行是必然的：它是汇合点 |
| 4 面板与工作台 | 5 | browser、remotes、panels+Metrics、toast/find/panetabs、workbench+terminal | 否 |
| 5 接线 + 收口 | 1 路接线 + 1 路文档 | 界面接线、五份文档 | 否 |
| 6 ShellBridge 对等 + 内嵌浏览器 | 0（子 Agent 全程禁用） | 壳命令、browser 模块、桥接、文档 | 是：本波次主 Agent 串行，非降级乃禁用 |
| 7 孤儿接线（生产组件） | 0（实测无可用子 Agent 类型） | Welcome/Files/document/cards/remote 等接线 | 是：环境不支持子 Agent，主 Agent 串行（见 B8） |

并行度 = 既独立又文件不重叠的任务数。不设上限：一波里有八个互不重叠的任务就
跑八个。实测本机支持**最多 8 路**并行子 Agent。

## 并行可用性记录（已更正）

### 旧结论（作废）

波次 1 曾记录「本环境没有子 Agent」，依据是每次派发都在跑起来之前被拒：

```
Agent(subagent_type: "general-purpose") -> Error: Agent type 'general-purpose' is disabled via disallowedTools configuration.
Agent(subagent_type: "Explore")         -> Error: Agent type 'Explore' is disabled via disallowedTools configuration.
Agent(no subagent_type)                 -> Error: Agent type 'general-purpose' is disabled via disallowedTools configuration.
```

**这条结论已经过时。** 它描述的是波次 1 那一刻的宿主策略，不是现在的能力。

### 现行结论

- 当前主机实测支持**最多 8 路并行子 Agent**，波次 2–5 的模块级任务确实是并行
  派发的。上面那张并行度表是实际值，不是理想值。
- 因此「主 Agent 被迫写业务模块代码」这条降级后果不再成立：业务模块由子 Agent
  产出，主 Agent 只做接线与验收。
- 波次 3 的接线仍然是串行 —— 这是真依赖，不是降级：`App.vue` 必须 import 每一屏，
  它是单个文件，汇合点无法拆分。

### 真实的新问题：子 Agent 后端 DNS 不稳定

并行能力可用，但**子 Agent 后端 `chickener.top` 曾多次 DNS 解析失败**，表现为
任务在派发或执行阶段报错。实测同一天不同时刻两种情况都出现过；正常时
`nslookup chickener.top` 返回 `104.19.128.100`、`104.17.32.100`（Cloudflare）。

这件事的危险不在「失败」，而在**失败与落盘不一致**：报错的任务可能已经把文件
写进了树。于是有两条纪律：

1. **不以子 Agent 的自我报告为依据。** 收口的每一个数字都是主 Agent 重新跑命令
   得到的（`vue-tsc`、`vitest`、`vite build`、`go build`、`go vet`、`cargo check`、
   `diff -rq`、`find | wc -l`、`grep -c`），不看报告里的数字。
2. **重试前先看文件系统。** 一个「失败」的任务在重派之前要确认它到底写了没有，
   否则会得到两份半成品或重复文件。

详见 `blockers.md` B7。

## 子 Agent 任务书模板

每份任务书都带这六个字段：

```
Scope:        这个 Agent 交付的那一件事
Input:        契约（类型/文件）与要读的 React 基准
Output:       它创建的文件，以及它必须跑的那条验证命令
Must not:     主 Agent 所有的那份清单，外加任何兄弟 Agent 的文件
Acceptance:   哪条命令退出 0 表示范围完成
Report:       范围 / 改了哪些文件 / 关键决策 / 命令与结果 / 遗留
```

一个做不完的子 Agent 报告阻塞，而不是越界编辑。波次继续；阻塞进 `blockers.md`。

### 波次 4/5 的实际分工（实测产出）

| 子 Agent 范围 | 产出 |
|---|---|
| BrowserPanel 族 | `ui/browser/` 7 个 `.vue` + 2 个测试 |
| Remotes 族 | `ui/remote/` 6 个 `.vue` + `hostbook.ts` + 1 个测试 |
| panels 族 + Metrics | `ui/panels/` 9 个面板 + `JobName.vue` + `kit.ts`、`derive.ts`、`jobElapsed.ts`；`ui/Metrics.vue` 归主 Agent |
| Toast / Find / PaneTabs | `ui/ToastStack.vue`、`ui/ToastRow.vue`、`ui/find/Find.vue`、`ui/PaneTabs.vue` |
| Workbench（5）+ Terminal（5） | `ui/workbench/` 5 个、`ui/terminal/` 5 个 |
| 接线（主 Agent） | `Pane.vue` 挂 Metrics / BrowserPanel / Find / WorkbenchPanel / TerminalView；`App.vue` 挂 ToastStack / PaneTabs / Remotes / RemoteAsk |

## 冲突裁决

- **同一文件被两个范围需要** → 后一个范围移到下一波次；不允许两人同时编辑。
- **子 Agent 需要改主 Agent 文件** → 报告所需契约（函数签名、prop 形状、
  provide 键），由主 Agent 在接线波次实现。D8 的 `settingsProps` 单 prop
  与 D9 的 `provide/inject` 键就是这样定下来的 —— 契约先定，20 个 section
  才可能并行产出。
- **子 Agent 之间语义打架**（同一个名字两种含义）→ 以 React 基准为准，主 Agent
  裁决并把结论写进 `decisions.md`。`Nav.onFiles` 的语义漂移就是这一类，见
  `blockers.md` C5。
- **守卫变红但代码「看起来对」** → 先怀疑守卫（`orphan` 曾经是空壳），但修守卫
  **不是**放宽集合。见 D11 与 `blockers.md` C6/C7。

## 上游指令冲突

`Downloads/DeepSeek-Reasonix-studio-RS/AGENTS.md` 要求串行子 Agent（「每次只能
调用一次」）。本迁移的目标明确要求最大并行（「并行是默认 … 上不封顶」），以目标
为准。这条覆盖记录在这里与 `decisions.md` D7，所以它是一条有理由的决策，而不是
一个没人注意到的矛盾。
