# profile-host（apps/reasonix-studio-host 任务书 —— 主 Agent 自做）

> 按编排原则，host 接线归主 Agent（相当于任务模板中的 crates/bin）。
> 本文件是接线清单与验收口径，不派子 Agent。

## 范围

Go 侧：`cmd/reasonix-studio-host`（main.go 626L + startup.go + owner.go 等）。
Rust 侧：`apps/reasonix-studio-host`（当前 exit-2 stub）。

## 必读

- `docs/contracts/host.md`（**全文**）、`docs/contracts/http.md` §2.6/§3（门/监听引用）
- `docs/profile-host.md`（本文件）、`docs/contract.md`（切片 S4）
- 子 Agent 产出的公开面：`crates/config`（host_serve_section/auto_workspace_dir）、
  `crates/serve`（build_router/gate/FindPage/graceful listener）、`crates/session`（Hub）
- Go 源：`cmd/reasonix-studio-host/{main,startup,owner}.go`

## 接线清单（host.md §1.7 顺序）

1. argv 优先级：schedule child 识别（`schedule exec` → 槽位保留：识别到即记日志
   exit 1，机器未迁见 OPEN_QUESTIONS §4）→ flag 解析（host.md §1.2 表；未迁移旗标
   解析照常接受、机器缺失按 §1.3 失败）。
2. 启动相位日志：`reasonix-studio-host: startup phase=<name> elapsed_ms=<n>`，
   12 相位顺序（HOST-004），boot.* 相位行。
3. stdin 租约：pipe/socket 判定（Windows：GetFileType/命名管道判定；无租约 =
   unsupervised）；EOF → 干净 exit 0（HOST-005/006；OPT-008 单一取消源）。
4. `FindPage`：nil → `reasonix-studio-host: no built page found; serving the kernel only`
   （HOST-032）。
5. assemble：config 加载（host_serve_section 清零）→ state 开会话面 → Hub 构造
   （OPT-008 的 CancellationToken 贯穿）。
6. bind：loopback listener + launch credential（getrandom 32B hex）+ `NewLoopbackGate`
   包 `serve::build_router`（HOST-010/017）。
7. provider-setup enable：本波次机器未迁，相位日志照发（host.md §1.7 第 8 步槽位），
   汇报标注 partial。
8. announce：一行 JSON `{version:1, origin, token}`，字段序 version/origin/token，
   行尾换行（HOST-008/009/011；act 行同 stdout，`std::sync::Mutex` 串行 OPT-009）。
9. serve：graceful drain（HOST-020）→ 退出码 0/1 表（HOST-003；OPT-019 ExitCode 归一）。
10. `-instance-id`（HOST-021）：`io.reasonix.studio.` + home 规范化 sha256[:8]。

## 验收（集成测试，crates/bin/tests 对应 apps/…/tests/）

- 握手首行 JSON + 字段序 + 64 hex token；每次启动 token 不同。
- 无 cookie → `403 loopback.unauthorized`；错 Host → `403 loopback.host_rejected`；
  变更无 Origin → `403 loopback.origin_rejected`；GET 空 Origin → 200。
- 配置 serve token 不开门（HOST-018 对照 host.go 的 TestHostDoesNotAdoptTheConfiguredServeToken）。
- 租约 EOF → exit 0 端口释放；信号 → exit 0；启动失败 → exit 1。
- act 行完整成行（并发写）。
- `-instance-id` 输出形状。

## 禁止

- 不依赖 `reasonix-cli`（HOST-033 结构守卫）。
- 不伪造未迁移机器的成功（provider-setup/tray/notify/schedule/update 槽位 = 相位日志 +
  明确 partial，见 contract.md §3）。

## OPT-015 证据留痕（m-7 复核补记）

相位日志的证据形态为复杂度分析 + 未量化标注：12 相位各一行，每行 O(1)
（Instant 差值 + 单次 writeln 到 LastLine），启动期一次性行不构成热点，
故按计划不设基准（数字无意义）；tracing 关闭路径也无需短路——相位行不走
tracing，直接写 stderr，成本即 12 次系统调用。
