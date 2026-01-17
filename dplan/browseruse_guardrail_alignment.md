# BrowserUse 对齐计划（Guardrail + AutoAct）

## 目标
让 SoulBrowser 在 LLM 正常和降级场景下都具备 BrowserUse 的 Search→AutoAct→Observe→Validate→Parse→Deliver 闭环，避免遇到 404/超时时退化为静态脚本。

## 任务拆分

1. **稳定 LLM Planner 主路径**
   - 为 `PlannerStrategy::Llm` 接入多 provider/本地限流，避免因为单个 key 限流而回退 rule plan。
   - 调整 `LlmPlanner` prompt，让 LLM 直接输出完整阶段图与 `thinking/evaluation/memory/next_goal`，减少 StageAuditor 的补丁操作。

2. **Rule Plan 注入 Guardrail 能力**
   - 在 `build_market_quote_recipe` 中使用 `StageContext::guardrail_queries()` 生成 `browser.search` payload，并写入 `planner_context.guardrail_keywords`。
   - 插入 `browser.search.click-result` step（带 `auto_act_refresh` metadata 和 `expected_url`），确保降级时也能自动筛选 SERP。
   - 为 Navigate/Observe 步骤设置 `EXPECTED_URL_METADATA_KEY`，让 guardrail URL 校验生效。

3. **Search/AutoAct 失败 → 刷新 & Replan**
   - 将 `browser.search` 30s 超时映射为 `search_no_results` blocker，调用 `augment_request_for_replan` 并携带失败 query/截图。
   - 复用 `guardrail_queries()` 提供的队列，让 Rule plan 下的 AutoAct 也能逐步降级 query，刷新 telemetry。
   - `browser.search.click-result` 超时或 URL mismatch 时，记录 `guardrail_query_used` 和 `search_context`，供 replanner/Serve 显示。

4. **Telemetry & Judge 对齐**
   - 扩展 `ExecutionMemoryEntry` 记录 Guardrail 刷新次数、query、Judge verdict，并在 `plans.json`/`executions.json` 呈现。
   - Serve/CLI 渲染 BrowserUse 风格的 Evaluate / NextGoal / Guardrail badge。

5. **测试 & 文档**
   - 新增集成测试模拟 “LLM rate-limit → Rule plan fallback → guardrail refresh → replan 成功” 流程。
   - 更新 `docs/agent_guardrails.md`、`dplan/browseruse_parity.md`，记录多 provider、Rule plan AutoAct、Search timeout → Replan 行为。

## 预期成果
- LLM 规划稳定，降级场景也拥有 AutoAct + Guardrail 刷新。
- SERP/Navigation 超时自动触发 replan，完成 BrowserUse 式自愈。
- Telemetry 呈现完整阶段与 Guardrail 细节，便于对比 BrowserUse 行为。
