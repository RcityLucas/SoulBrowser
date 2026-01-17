use agent_core::plan::{AgentPlan, AgentTool, AgentToolKind};
use agent_core::planner::PlanStageKind;
use agent_core::WaitMode;
use serde_json::json;
use std::collections::HashMap;

use super::{stage_overlay, StageStrategy, StrategyApplication, StrategyInput, StrategyStep};

#[derive(Debug, Default)]
pub struct AutoEvaluateStrategy;

impl AutoEvaluateStrategy {
    pub fn new() -> Self {
        Self::default()
    }
}

impl StageStrategy for AutoEvaluateStrategy {
    fn id(&self) -> &'static str {
        "auto_evaluate"
    }

    fn stage(&self) -> PlanStageKind {
        PlanStageKind::Evaluate
    }

    fn apply(&self, input: &StrategyInput<'_>) -> Option<StrategyApplication> {
        let Some(source_step_id) = latest_observation_step(input.plan) else {
            return None;
        };
        let tool = AgentTool {
            kind: AgentToolKind::Custom {
                name: "agent.evaluate".to_string(),
                payload: json!({
                    "source_step_id": source_step_id,
                    "message": "评估最近一次观察结果",
                }),
            },
            wait: WaitMode::None,
            timeout_ms: Some(2_000),
        };
        let step = StrategyStep::new("评估页面状态", tool)
            .with_detail("自动评估最近一次观察结果")
            .with_agent_state(json!({
                "thinking": "检查页面是否符合目标字段与域名",
                "evaluation": "若不符合，将触发 guardrail 并重新规划",
                "next_goal": "若校验通过，进入解析/交付阶段"
            }));
        Some(StrategyApplication {
            steps: vec![step],
            note: Some("自动追加评估步骤".to_string()),
            overlay: Some(stage_overlay(
                PlanStageKind::Evaluate,
                self.id(),
                "applied",
                "🧐 评估当前页面状态",
            )),
            vendor_context: HashMap::new(),
        })
    }
}

fn latest_observation_step(plan: &AgentPlan) -> Option<String> {
    plan.steps.iter().rev().find_map(|step| {
        if matches!(step.tool.kind, AgentToolKind::Custom { ref name, .. }
                if name.eq_ignore_ascii_case("data.extract-site")
                    || name.eq_ignore_ascii_case("market.quote.fetch"))
        {
            Some(step.id.clone())
        } else {
            None
        }
    })
}
