use agent_core::{
    plan::{AgentTool, AgentToolKind},
    planner::PlanStageKind,
    WaitMode,
};
use serde_json::json;
use std::collections::HashMap;

use super::{
    latest_observation_step, stage_overlay, StageStrategy, StrategyApplication, StrategyInput,
    StrategyStep,
};
use crate::agent::guardrails::{derive_guardrail_domains, derive_guardrail_keywords};

#[derive(Debug, Default)]
pub struct TargetGuardrailStrategy;

impl TargetGuardrailStrategy {
    pub fn new() -> Self {
        Self::default()
    }
}

impl StageStrategy for TargetGuardrailStrategy {
    fn id(&self) -> &'static str {
        "target_guardrail"
    }

    fn stage(&self) -> PlanStageKind {
        PlanStageKind::Validate
    }

    fn apply(&self, input: &StrategyInput<'_>) -> Option<StrategyApplication> {
        let Some((_, observation_id)) = latest_observation_step(input.plan) else {
            return None;
        };
        let mut keywords = if !input.context.guardrail_keywords.is_empty() {
            input.context.guardrail_keywords.clone()
        } else {
            derive_guardrail_keywords(input.request)
        };
        if keywords.is_empty() {
            keywords.push(input.request.goal.clone());
        }

        let mut allowed_domains = if !input.context.guardrail_domains.is_empty() {
            input.context.guardrail_domains.clone()
        } else {
            derive_guardrail_domains(input.request)
        };
        if keywords.is_empty() && allowed_domains.is_empty() {
            return None;
        }
        if allowed_domains.is_empty() {
            allowed_domains.push("www.baidu.com".to_string());
        }
        let detail = format!(
            "校验页面是否包含 {} 并来自可信域名",
            keywords
                .get(0)
                .cloned()
                .unwrap_or_else(|| input.request.goal.clone())
        );
        let tool = AgentTool {
            kind: AgentToolKind::Custom {
                name: "data.validate-target".to_string(),
                payload: json!({
                    "source_step_id": observation_id,
                    "keywords": keywords,
                    "allowed_domains": allowed_domains,
                    "expected_status": 200,
                }),
            },
            wait: WaitMode::None,
            timeout_ms: Some(3_000),
        };
        let step = StrategyStep::new("验证目标页面", tool).with_detail(detail);
        Some(StrategyApplication {
            steps: vec![step],
            note: Some("追加 data.validate-target 确认页面域名/关键词".to_string()),
            overlay: Some(stage_overlay(
                PlanStageKind::Validate,
                self.id(),
                "applied",
                "🛡️ 追加目标校验步骤",
            )),
            vendor_context: HashMap::new(),
        })
    }
}
