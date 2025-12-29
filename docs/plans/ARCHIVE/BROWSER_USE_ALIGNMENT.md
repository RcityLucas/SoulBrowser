# Browser Use 对齐追踪（Archive）

> 参考资料：[browser-use/browser-use](https://github.com/browser-use/browser-use) - README、Agents.md、系统提示，聚焦其“先计划、再观察、再行动、最后交付”的显式分阶段执行体验。

## 🎯 背景
- Browser Use 将“计划-执行-审核”作为一等公民，计划卡片会展示每个阶段的策略与成败，同时配套有严格的 Judge 体系。
- SoulBrowser 需要提供等价的透明度：系统生成的计划要解释每个阶段的来源，并且在执行前就给出缺失阶段/输出的提示。
- 本文档归档与 Browser Use 的差距及阶段性补齐项，供后续 L8 策略/规划讨论引用。

## ✅ 已完成 (2025-02)
1. **阶段覆盖可视化**  
   - `StageAuditor` 现在会针对 stage graph（navigate/observe/act/parse/deliver）生成覆盖日志：
     - 已存在的阶段记为 `existing`。
     - 通过策略/占位自动补齐的阶段会记录 `auto_strategy` / `placeholder`。
     - 无法自动满足的阶段（目前主要是 act）会明确标记为 `missing`。
   - Web Console / CLI 均可看到这些 overlay，效果与 Browser Use 的“todo & plan scoreboard”一致。
2. **严格校验逻辑**  
   - `PlanValidator::strict` 追加 Browser Use 风格的规则：
     - 缺少 observation -> parse -> deliver 的 DOM 计划会被拒绝。
     - 有结构化输出需求但没有 `data.deliver.structured` 会被拒绝。
     - Weather 关键词会强制 `data.parse.weather + weather deliver`。
     - 需要面向用户回答的请求（result keyword / informational intent）必须包含 `agent.note` 或结构化交付。
   - 对应的单元测试 (`tests/plan_validator.rs`) 也补齐，保证行为与 Browser Use 的“judge gate”一致。
3. **天气/查询意图抽取**  
   - Weather subject 提取/编码逻辑（`weather.rs`）现在会：
     - 去除「查询/帮我查」等动词前缀。
     - 去除 trailing `天气/气温/weather/forecast` 描述，统一附加 `" 天气"`。
     - URL 编码统一使用 `%20`，避免 Browser Use pipeline 因 `+` 号而误判。

## 🔜 待办
1. **Act 阶段策略**：目前仅报告缺失，需要结合视觉/插件信号补全“表单填写/多步点击”策略，参考 Browser Use 的 todo.md 驱动方式。
2. **Judge 对齐**：Browser Use 的 judge 会基于截图+轨迹进行二次判定；SoulBrowser 需在 `l6-privacy`/`l6-timeline` 中补齐裁判逻辑并输出 verdict。
3. **记忆/文件工具**：Browser Use 会动态生成 todo.md、文件存储等；后续需结合 `memory-center` 与 `agent.note` 输出策略展开。
4. **Cloud/Sandbox 模式**：Browser Use 的 cloud session/sandbox 概念需要在 `docs/AI_BROWSER_EXPERIENCE_PLAN.md` 后续章节中补入等价治理方案。

## 📚 参考
- Browser Use README + demos：多阶段截图 + 结构化工具说明。
- [Agents.md（system prompt 摘要）](https://docs.browser-use.com/llms-full.txt)：强调“先写计划、再执行”的提示范式。
- SoulBrowser 现有文档：`docs/AI_BROWSER_EXPERIENCE_PLAN.md`、`docs/SERVE_ARCHITECTURE.md`。
