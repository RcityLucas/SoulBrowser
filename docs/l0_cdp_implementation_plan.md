# L0 · 可执行实现待办（CDP + 监控）

> 目标：补齐 `crates/cdp-adapter` 与 `src/l0_bridge.rs`，使其对接真实 CDP 事件流、网络探针，并满足监控/验收需求。

## 1. 依赖与准备
- [x] 引入 `chromiumoxide`、`rand` 等依赖（已完成 `cargo fetch`）。
- [ ] 确认优先使用的浏览器（Chromium/Chrome）与运行参数，准备调试配置。例如：可执行路径、user-data-dir、headless/显示输出、代理设置。
  - ⚠️ 当前默认优先读取环境变量 `SOULBROWSER_CHROME`；若未设置则依赖系统 PATH 或 chromiumoxide 的自动检测。建议显式安装 Chrome/Chromium 并通过 `SOULBROWSER_CHROME` 指定可执行路径，避免因找不到浏览器而出错。
- [ ] 准备本地或 CI 环境：需要可运行的 Chrome 和 WebSocket 访问权限；考虑沙箱与权限限制。

## 2. Transport 实现（`crates/cdp-adapter/src/transport.rs`）
- [x] 新建 `ChromiumTransport` 结构体：
  - 启动/连接浏览器；
  - 保存 `Browser`, `Handler`（chromiumoxide 链路）、channel senders/receivers；
  - 使用 `Handler::new`、`Connection::connect` 处理 websocket 连接，支持外部既有实例。
- [x] 实现 `CdpTransport` trait：
  - `start()`：启动事件循环，订阅 `Handler` 的事件和响应；
  - `next_event()`：返回统一 `TransportEvent`（`method` + `params` JSON）；
  - `send_command()`：生成 command id、发送并 async 等待响应，超时/错误转换为 `AdapterError`；可利用 `CommandMessage`、`to_command_response`；
  - 维护 inflight 请求映射（如 `DashMap<u64, oneshot::Sender<Value>>`）。
- [ ] 自愈：检测断线/异常时重连；
  - 监听 `Connection` 错误事件，重建浏览器/连接。

## 3. Adapter 层（`crates/cdp-adapter/src/adapter.rs`）
- [x] 扩展 `CdpAdapter::start()`：
  - 使用 `ChromiumTransport` 替换 `NoopTransport`；
  - 启动事件循环，将 `TransportEvent` 解析为 `RawEvent`（映射 `Page.lifecycle`, `Network.*`, `Runtime.exception` 等）；
  - 更新 `Registry`：注册新 session/page/frame；
  - 使用 `network_tap_light` retrofit（或将网络摘要发送到 bridge）。
- [x] `handle_event()`：
  - 解析 `method` 字符串映射到具体 `RawEvent`；
  - 维护 `PageId ↔ Target` 映射（`registry`）；
  - 将 RawEvent 推送到 `bus`。
- [x] 补充常用命令：navigate/click/type/wait/screenshot → 生成具体 `chromiumoxide_cdp` 命令参数。
- [x] `wait_basic`：映射 `WaitGate` 到 `chromiumoxide` 的等待 API（如 `Page::wait_for_navigation`、`Runtime::evaluate` 等）。
- [x] 错误映射：`chromiumoxide::error::CdpError` -> `AdapterError`（含 retriable 标识）。

## 4. 网络探针整合
- [ ] 在 `handle_event()` 中聚合 `Network.requestWillBeSent`/`Network.response*` 等事件；
  - 使用滑窗统计生成 `RawEvent::NetworkSummary`；
  - 或直接转发 CDN tap 事件（若后续有单独模块）。
- [ ] 给 `NetworkTapLight` 提供真实摘要输入：
  - `handles.network_tap.update_snapshot(page, snapshot)`；
  - 与 `L0Bridge` 的 `NetworkSummary` 事件保持一致，避免重复。

## 5. L0 ↔ L1 桥接完善（`src/l0_bridge.rs`）
- [ ] 当前 bridge 已模拟映射：需要从真实事件取代 `TapPageId::new()` 的生成逻辑，改为 CDP target id ↔ Registry `PageId` ↔ network tap id 映射；
- [ ] 支持多 session/page 并发，创建/销毁时更新 `mapping`/`tap_mapping`；
- [ ] 处理 frame attach/detach 时的 parent frame/child 结构。

## 6. 监控与指标
- [ ] 扩展 scheduler metrics，加入请求耗时、失败原因；
- [ ] 为 CDP adapter 增加基础 metrics（命令 P95、事件总数、重连次数），输出到 `tracing` 或 metrics crate；（当前已接入命令计数/成功率/耗时累计，待补 P95 与重连统计）
- [ ] 接入可选的 Soulbase-observe/Prometheus exporter，输出 `scheduler`/`registry`/`adapter` 关键指标。

## 7. 测试计划
- [ ] 单元测试：
  - Transport 层使用 mocked handler/connection（chromiumoxide 提供 test harness）；
  - Parser/聚合逻辑的独立测试（RawEvent mapping）。
- [ ] 集成测试：
  - 在本地/CI 启动 headless Chrome，执行 navigate/click 等脚本，验证 RawEvent 与 Registry/State Center 更新；
  - Validate network summary -> registry health -> state center 事件；
  - 执行 `tests/l1_e2e.rs`，移除 `sleep` 改为事件同步等待。

## 8. 验收
- [ ] 对照 `docs/l1_acceptance_checklist.md` 完成验证：
  - 功能：CDP 命令 OK、取消路径 OK、PageHealth 更新 OK；
  - 性能：记录调度吞吐/CDP 调用指标；
  - 故障注入：断线/超时/取消 -> 事件与指标可见；
  - 安全：策略 override 白名单、State Center 数据脱敏。
- [ ] 更新 `docs/l1_operations.md` 操作说明、运维策略。
- [ ] CI 增加端到端测试任务（可 conditionally run 在支持浏览器的 runner）。

---

完成上述步骤后，即可 claim “真实 L0 事件源集成 + 监控 + 验收”已达成。

## 9. 里程碑拆解

> 可根据以下清单创建任务卡，按顺序实施；每个里程碑完成后再推进下一阶段。

### Milestone A：CDP 连接与基础命令
- [ ] `transport.rs` 实现 `ChromiumTransport`：启动/连接、事件循环、命令响应映射、自愈。
- [ ] `adapter.rs` 注册真实 transport，解析关键 CDP 事件为 `RawEvent`，维护 target ↔ page/frame 映射。
- [ ] 导航/点击/输入/截图命令调用 `chromiumoxide_cdp` 类型化参数；实现 `wait_basic` 初始 gate。
- [ ] 本地验证：启动 headless Chrome，执行简单脚本，确认事件总线与 Registry 更新。

### Milestone B：网络探针与桥接
- [x] 聚合 `Network.*` 事件为 `RawEvent::NetworkSummary`，写入 `NetworkTapLight`。
- [ ] `L0Bridge` 使用真实 TargetId/TapId 映射，多页面并发下正确创建/关闭。
- [ ] 触发 `RegistryAction::PageHealthUpdated` 并写入 State Center；调整 `tests/l1_e2e.rs` 使用事件通知而非 `sleep`。

### Milestone C：监控与取消闭环
- [ ] 扩充 scheduler/adapter metrics（耗时、失败原因、重连次数），暴露 snapshot / exporter。
- [ ] 调度取消（action/call/task）在指标、State Center、日志中可见；完善端到端测试覆盖。

### Milestone D：验收执行
- [ ] 按 `docs/l1_acceptance_checklist.md` 跑通功能/性能/故障/安全场景。
- [ ] 编写自动化脚本（性能基线、故障注入、日志校验），生成验收报告。

### Milestone E：文档与 CI 落地
- [ ] 更新 `docs/l1_operations.md`、本计划文档，记录实操指南与监控策略。
- [ ] CI 新增端到端任务（需要浏览器支持的 runner），默认执行离线 `cargo test --all` + 集成验证。
- [ ] 汇总验收结果，提交最终交付记录。

完成所有里程碑后，再进行最终验收签字。
