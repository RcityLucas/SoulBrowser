# 插件生态文档中心

## 用途
- 汇总 L7/L8 插件与外部接口的规范、审核流程、安全策略。
- 支持阶段3 的生态扩展以及后续运营。

## 建议结构
1. `manifest_spec.md` —— 插件声明、权限模型、版本规范。
2. `review_process.md` —— 自动/人工审核步骤、所需材料、决策标准。
3. `sandbox_runtime.md` —— 沙箱架构、资源限制、监控指标。
4. `webhooks.md` —— 外部触发协议、认证、重试策略。
5. `security_playbook.md` —— 风险分类、应急响应。
6. `browser_use_gap.md` —— Browser Use 生态能力（REST/SDK、示例任务、审批流程）对比表。

## 下一步
- 根据阶段3 技术文档编写初版 Manifest 与审核流程。
- 与策略中心、治理团队共建安全模型与审批模板。
- 准备插件开发者指南与示例仓库结构。
- 定期更新对标表，量化 REST/SDK 覆盖率、示例任务数量、审批 SLA，与 Browser Use 对齐。
