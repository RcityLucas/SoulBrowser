# L1 统一内核 · 验收检查清单

## 功能验证
- [ ] Policy Center 级联加载：验证内建 + 文件 + 环境/CLI 覆盖及 TTL override。
- [ ] Scheduler 调度流：提交、完成、取消（Action/call/task）路径均可观测并落入 State Center。
- [ ] Registry 生命周期：CDP RawEvent 推动 session/page/frame 变更；network summary 写入 `PageHealthUpdated`。
- [ ] State Center 快照：`state-center-snapshot.json` 包含全局统计与 session/page/task/action 事件。

## 性能基线
- [ ] 调度吞吐在预期范围内（待设定数值）。
- [ ] State Center ring 大小 / 内存占用监控。

## 故障注入
- [ ] 调度取消 → 状态中心出现 `cancelled` 事件。
- [ ] 工具超时 / 重试 → `DispatchStatus::Failure` 正确记录。
- [ ] CDP 连接丢失 → Registry 触发恢复逻辑并记录事件。

## 安全与合规
- [ ] 覆盖白名单：仅允许 override 安全字段（CI 校验）。
- [ ] State Center 快照脱敏：敏感数据裁剪/不可逆。
- [ ] Policy provenance：`policy.provenance` 可追溯来源。

## 自动化
- [ ] CI 执行 `CARGO_NET_OFFLINE=true cargo test --all`。
- [ ] 集成测试 `tests/l1_e2e.rs` 通过，确认端到端路径。
