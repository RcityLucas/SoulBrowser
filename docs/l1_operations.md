# L1 统一内核 · 运维指引概览

## Policy Center
- 支持级联加载顺序：Builtin → 文件(`policy.yaml`) → 环境变量（`SOUL_POLICY__*` / `SOUL_POLICY_OVERRIDE_JSON`） → CLI (`SOUL_POLICY_CLI_OVERRIDES`) → Runtime TTL 覆盖。
- 使用 `soulbrowser policy show --json` 查看当前快照与 provenance，`soulbrowser policy override --path ... --value ... --ttl` 提交运行时覆盖。
- 若需要在代码中维持策略粘附，可调用 `PolicyCenter::guard().await` 获取 `PolicyGuard` 并在同一 rev 下消费。

## Scheduler
- 新增取消接口：
  - `Dispatcher::cancel(action_id)` 取消指定 Action。
  - `Dispatcher::cancel_call(call_id)` 基于外部幂等键撤销排队任务。
  - `Dispatcher::cancel_task(task_id)` 批量取消同一 Task 关联的任务。
- 内部 metrics 提供 `scheduler::metrics::snapshot()`，输出 enqueued / started / completed / failed / cancelled 计数，可向上游曝光。

## Registry
- `RegistryImpl::apply_network_snapshot(page_id, snapshot)` 可注入 L0 网络摘要并刷新 `PageHealth`；`ingest` 模块新增 `NetworkSummary` 事件分支以便通过事件总线传递。
- `PageCtx` 持有实时健康信息，`RegistryAction::PageHealthUpdated` 事件将写入 State Center。
- `src/l0_bridge.rs` 订阅 CDP RawEvent 与 network tap 摘要，将 L0 生命周期/健康事件桥接到 Registry/State Center（暂以 `cli-default` 会话为宿主）。

## State Center
- 事件按照 session/page/task/action 四类 ring 缓存，可通过 `InMemoryStateCenter::recent_*` 快速查询。
- `write_snapshot` 生成的 JSON 包含全局统计与各作用域事件条目数，默认输出至 `state-center-snapshot.json`。
- 当策略 `features.state_center_persistence` 打开后，后台任务每 5 秒落盘一次快照。

## 运维建议
- 建议在部署环境中设置 `SOUL_POLICY_OVERRIDE_JSON` 统一注入灰度策略，运行期再用 CLI 追加临时覆盖。
- Cancel API 与新的健康摘要事件已写入 State Center，可通过 `soulbrowser state history` 或直接读取快照进行排障。
- 若需要导出 metrics，可在上层调用 `scheduler::metrics::snapshot()` 并接入现有指标系统。
