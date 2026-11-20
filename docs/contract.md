# 契约冻结（contract.md）

> 波次 17 迁移窗口使用。**权威契约已存在且冻结**：`docs/contracts/*.md`（13 份，M1，
> 2026-11-15 自 Go 源冻结）+ `crates/contract/src/eventwire_manifest.json`（Go 反射权威）
> + `docs/parity/LEDGER.md`（563 行验收账本）。子 Agent 只读这些权威文件；
> 本文件是**本波次切片的索引与裁剪声明**，不复述、不改写权威契约。

## 1. 权威契约索引

| 契约 | 文件 | 本波次相关性 |
|---|---|---|
| HTTP 面（路由/鉴权/信封/绑定） | docs/contracts/http.md | **核心**（§2.6 门、§4 信封、§5 SSE 帧、§7 形状） |
| SSE 词汇与编号 | docs/contracts/sse.md | **核心**（§1..§4 kind 表/信封/编号/订阅） |
| host 握手与生命周期 | docs/contracts/host.md | **核心**（§1..§6） |
| 配置 | docs/contracts/config.md | **核心**（host 启动子集） |
| 会话格式 | docs/contracts/session-format.md | 间接（消费 state crate，不新写） |
| provider 消息 | docs/contracts/provider-message.md | 已完成（PMSG 20/20 done） |
| tools / mcp / ext / acp / cli / paths / state | 各自文件 | 本波次不触 |

线缆词汇权威：`crates/contract/src/eventwire_manifest.json`（67 类型，golden 门
`eventwire_parity.rs` 持有）；SSE 事件 JSON 的 Rust 类型**必须**从
`crates/contract::eventwire` 复用，禁止在 serve/session 重新声明。

## 2. 本波次切片（「哪些行为算完成」）

每条切片完成 = 对应账本行翻 done（附测试名/命令）+ `cargo xtask parity --check` 绿。

| 切片 | crate | 契约锚点 | 涉及账本行 |
|---|---|---|---|
| S1 配置启动子集 | crates/config | config.md（加载/分层/serve 段/desktop 键/auto workspace/language） | CONFIG 子集 + HOST-018 |
| S2 hub 读侧 + 树 | crates/session | http.md §1.2（/tree、/runtimes 列表）；session-format（读模型复用） | ROUTE 树子集、SESS 列表相关行 |
| S3 门/信封/SSE 帧/路由面 | crates/serve | http.md §2.6/§2.9/§4/§5、sse.md §1..§4 | ROUTE-008/013/014/017/020/021/022/023/024/027/028/029(前半)/032(子集)、SSE-001..008(子集) |
| S4 host 启动面 | apps/reasonix-studio-host | host.md §1/§2/§4/§5.1/§6.7 | HOST-001..011/014/017/018/020/021/032 |

## 3. 本波次明确裁剪（路由不注册、机器不伪造）

裁剪遵循一个原则：**没实现的机器不注册其路由**（Go 的 HubOptions nil 即不注册，
http.md §0 的先例），绝不返回假成功。裁剪面：

- password 鉴权（ROUTE-011）、device 门与 `/pair`（ROUTE-015/016）、hostOnlyRoutes 三点执行（ROUTE-016）
- events mux 家族 `/events/mux*`（ROUTE-029 后半）
- wire log / `GET /trajectory`（ROUTE-026）
- sink 包装层：Coalesce（SSE-009）、CostQuoteSink（SSE-011）、NoticeLedger（SSE-012）、
  MarkEmbeddedDiffs（SSE-013）、能力注册表（SSE-010）——broadcaster 直通
- `/tree/events` 与 `/tree/events/replay` 的 live 流（树变更推送依赖后续波次的
  watcher 机器；本波次 /tree 快照可达）
- host：tray/notify/schedule ticker/update/strip-package-grants 机器（HOST-015/016/022..031/034..038）
  ——argv 槽位保留（§1.1/§1.2 优先级），未迁移机器按启动失败 exit 1 记日志
- agent 写侧全部路由（/submit /cancel /approve /new …）：不注册（无 runtime 机器），
  routeDefault 的 503 `hub.no_runtime_open` 是唯一合法出口

## 4. 层间冻结（子 Agent 不得改签名）

 crates 依赖方向沿用 `docs/kernel-rs/AGENTS.md 4.2`（xtask deps --check 持有）：

```
serve → session → state; serve/config/session → contract/core; host(app) → serve/session/config
```

本波次新增冻结边界（写入骨架、由 `cargo xtask deps --check` 与编译器持有）：

1. `crates/session::Hub`（读侧）：
   - `fn tree(&self) -> WorkspaceTreeSnapshot`
   - `fn runtimes(&self) -> Vec<RuntimeSummary>`
   - `fn sessions(&self, workspace: &Path) -> Vec<SessionRow>`
   - `fn open_runtime(&self, req: OpenRuntimeRequest) -> Result<RuntimeSummary, HubRefusal>`（读侧构建：返回 HubRefusal::UnsupportedInThisBuild）
   - `fn broadcaster(&self, runtime: RuntimeId) -> Option<Arc<BroadcasterHandle>>`（写侧机器未在时 None）
2. `crates/serve::gate`（loopback 门）：`LoopbackGate::{new, check_http}` 返回
   `GateDecision::{Allow, Reject(RefusalCode)}`，拒绝码恒为
   `loopback.host_rejected / loopback.origin_rejected / loopback.unauthorized / loopback.misconfigured`。
3. `crates/serve::refuse`：`Reason { code, error, params }` + `RefuseError` 分类枚举
   （http.md §4.2 的 writeErr 映射序）。
4. `crates/serve::broadcaster`：`Broadcaster::{emit, subscribe(SubscriptionMode), replay(after), watermark, reset_session}`，
   `SubscriptionMode::{From(after), InitialNotices, Attached, Resume(after)}`（sse.md §4 的五模式去掉无参 Subscribe=From(0) 的别名重复）。
5. `crates/config::Config`：`load(layered_paths) -> Result<Config, ConfigError>` +
   `ServeSection::{auth_mode, token, password_hash, behind_proxy}`（HOST-018 的清零辅助
   `host_serve_section()`）+ `auto_workspace_dir()` + `desktop` getters。

trait/struct 签名一经本文件声明即冻结；调整需回到主 Agent 改本文件并同步两侧。
