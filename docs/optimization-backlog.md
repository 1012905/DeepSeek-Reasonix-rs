# 优化积压（optimization-backlog.md）

> 收纳两类内容：L3（会改变外部契约，**禁止在本窗口实现**）与后续波次范围
> （本波次裁剪面）。每条注明契约锚点与解锁条件。

## L3（契约级变更 —— 一律不实现）

| ID | 候选变更 | 为什么拒绝 | 契约锚点 |
|---|---|---|---|
| L3-001 | 统一 SSE 事件 JSON 的 snake/camel 混用（Guardian 段 snake_case） | wire 稳定词汇，改名是破坏性变更 | sse.md §9.1 |
| L3-002 | 去掉 `truncated` 与 `bound.kind==truncated` 的重复字段 | 兼容旧日志投影，双字段是冻结的 | sse.md §9.2 |
| L3-003 | turn_done 的 `err` 与 HTTP 信封的 `error` 统一 | 同词不同键均为冻结面 | sse.md §9.3、http.md §4.2 |
| L3-004 | 把 404 `request.not_found` 收敛为统一 `{kind,name}` 之外的新信封 | 拒绝码是稳定面，改码等于改路由 | http.md §4.2/4.3 |
| L3-005 | loopback 门允许 query token（方便调试） | 「cookie only」是门的安全属性 | http.md §2.6 |
| L3-006 | 移除 vite 代理的三个 stale prefix（/fork 等） | 前端面冻结，另行处置 | http.md §6/§8.1 |
| L3-007 | 配置未知键从（Go 现行为）改为忽略 | 键面冻结 | config.md、CONFIG 行 |

## 后续波次范围（本波次裁剪面，非 L3 —— 实现后即可做）

| ID | 项 | crate | 解锁条件 |
|---|---|---|---|
| B-001 | password 鉴权模式（bcrypt12/HMAC 会话/限速） | serve | 本波次后任意波次；契约 http.md §2.3 |
| B-002 | device 门 + `/pair` + hostOnlyRoutes 三点执行 | serve | device 配对机器迁移 |
| B-003 | events mux（多 pane 单连接） | serve | http.md §5 mux 家族 |
| B-004 | wire log + /trajectory | serve | ROUTE-026 |
| B-005 | sink 包装层：Coalesce/CostQuoteSink/NoticeLedger/MarkEmbeddedDiffs/能力注册表 | serve | sse.md §5（SSE-009..013） |
| B-006 | /tree/events live 流（watcher 机器） | serve/session | 树 watcher（platform 迁移） |
| B-007 | provider 客户端（internal/model → crates/provider，reqwest+rustls） | provider | PMSG 已 done，客户端随写侧波次 |
| B-008 | agent 循环（internal/runtime → crates/agent） | agent | 写侧总闸：/submit /cancel /approve 等 |
| B-009 | tools（internal/tools → crates/tools）+ MCP | tools | TOOL 57 行 + MCP 38 行 |
| B-010 | safety（internal/safety → crates/safety） | safety | SAFETY 面（沙箱/审批/egress） |
| B-011 | ext（internal/ext → crates/ext） | ext | EXT 46 行 |
| B-012 | platform（internal/platform → crates/platform） | platform | PATHS 36 行 + 各平台机器 |
| B-013 | tui / acp / cli 全量 | tui/acp/cli | ACP 46 行 + CLI 49 行 + §7 旗标矩阵 |
| B-014 | assembly boot（internal/assembly → crates/assembly） | assembly | 依赖 B-007..B-010 |
| B-015 | host 平台机器：tray/notify/schedule ticker/update/strip-package-grants | host | platform 迁移后 |
| B-016 | config 全键面（providers/presets/memory/hooks/...29.5k 行） | config | CONFIG 35 行剩余 |
| B-017 | cutover：账本收敛 → parity 签收 → Go 归档删除（_go_archive/ → rm） | 仓库级 | RUST_MIGRATION 决策 5 / B15 |

## 已识别但未立项的观察（来自 Go 侧通读）

- `internal/frontend/serve/serve.go`（959 行）单文件承载 Server 生命周期 + 路由 + 若干
  handler：Rust 侧按模块拆分（gate/listener/page/refuse/router），不复刻巨文件。
- Go config 包 29.5k 行单包：Rust 侧按 config.md 的段拆模块，避免同型巨包。
- hub_projects_benchmark 的 Go 基线数字：后续波次补跑 `go test -bench` 记录，作为
  criterion 对照的权威基线（本波次标「未量化」）。
