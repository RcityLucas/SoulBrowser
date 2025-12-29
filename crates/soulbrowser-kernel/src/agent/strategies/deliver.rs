use agent_core::plan::{AgentPlan, AgentTool, AgentToolKind};
use serde_json::json;

use super::{stage_overlay, StageStrategy, StrategyApplication, StrategyInput, StrategyStep};

#[derive(Debug, Default)]
pub struct StructuredDeliverStrategy;

impl StructuredDeliverStrategy {
    pub fn new() -> Self {
        Self::default()
    }
}

impl StageStrategy for StructuredDeliverStrategy {
    fn id(&self) -> &'static str {
        "structured"
    }

    fn stage(&self) -> agent_core::planner::PlanStageKind {
        agent_core::planner::PlanStageKind::Deliver
    }

    fn apply(&self, input: &StrategyInput<'_>) -> Option<StrategyApplication> {
        let Some((_, parse_id, schema)) = latest_parse_step(input.plan) else {
            return None;
        };
        let schema_value = schema.unwrap_or_else(|| "generic_observation_v1".to_string());
        let tool = AgentTool {
            kind: AgentToolKind::Custom {
                name: "data.deliver.structured".to_string(),
                payload: json!({
                    "source_step_id": parse_id,
                    "schema": schema_value,
                    "artifact_label": format!("structured.{}", schema_value),
                    "filename": format!("{}.json", schema_value),
                }),
            },
            wait: agent_core::WaitMode::None,
            timeout_ms: Some(3_000),
        };
        let step = StrategyStep::new("交付结构化结果", tool);
        Some(StrategyApplication {
            steps: vec![step],
            note: Some("生成结构化交付步骤".to_string()),
            overlay: Some(stage_overlay(
                agent_core::planner::PlanStageKind::Deliver,
                self.id(),
                "applied",
                "📦 输出结构化结果",
            )),
        })
    }
}

#[derive(Debug, Default)]
pub struct AgentNoteStrategy;

impl AgentNoteStrategy {
    pub fn new() -> Self {
        Self::default()
    }
}

impl StageStrategy for AgentNoteStrategy {
    fn id(&self) -> &'static str {
        "agent_note"
    }

    fn stage(&self) -> agent_core::planner::PlanStageKind {
        agent_core::planner::PlanStageKind::Deliver
    }

    fn apply(&self, input: &StrategyInput<'_>) -> Option<StrategyApplication> {
        let summary = input
            .request
            .intent
            .primary_goal
            .clone()
            .unwrap_or_else(|| input.request.goal.clone());
        let tool = AgentTool {
            kind: AgentToolKind::Custom {
                name: "agent.note".to_string(),
                payload: json!({
                    "title": "自动总结",
                    "detail": summary,
                }),
            },
            wait: agent_core::WaitMode::None,
            timeout_ms: Some(2_000),
        };
        let step = StrategyStep::new("生成总结", tool);
        Some(StrategyApplication {
            steps: vec![step],
            note: Some("补充 agent.note 输出".to_string()),
            overlay: Some(stage_overlay(
                agent_core::planner::PlanStageKind::Deliver,
                self.id(),
                "applied",
                "📝 生成文字总结",
            )),
        })
    }
}

fn latest_parse_step(plan: &AgentPlan) -> Option<(usize, String, Option<String>)> {
    plan.steps
        .iter()
        .enumerate()
        .rev()
        .find_map(|(idx, step)| match &step.tool.kind {
            AgentToolKind::Custom { name, payload } if is_parse_tool(name) => {
                let schema = schema_for_parse_tool(name, payload);
                Some((idx, step.id.clone(), schema))
            }
            _ => None,
        })
}

fn is_parse_tool(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "data.parse.generic"
            | "data.parse.market_info"
            | "data.parse.news_brief"
            | "data.parse.weather"
            | "data.parse.twitter-feed"
            | "data.parse.facebook-feed"
            | "data.parse.linkedin-profile"
            | "data.parse.hackernews-feed"
            | "data.parse.github-repo"
    )
}

fn schema_for_parse_tool(name: &str, payload: &serde_json::Value) -> Option<String> {
    let key = name.trim().to_ascii_lowercase();
    match key.as_str() {
        "data.parse.generic" => payload
            .as_object()
            .and_then(|obj| obj.get("schema"))
            .and_then(|value| value.as_str())
            .map(|s| s.to_string()),
        "data.parse.market_info" => Some("market_info_v1".to_string()),
        "data.parse.news_brief" => Some("news_brief_v1".to_string()),
        "data.parse.weather" => Some("weather_report_v1".to_string()),
        "data.parse.twitter-feed" => Some("twitter_feed_v1".to_string()),
        "data.parse.facebook-feed" => Some("facebook_feed_v1".to_string()),
        "data.parse.linkedin-profile" => Some("linkedin_profile_v1".to_string()),
        "data.parse.hackernews-feed" => Some("hackernews_feed_v1".to_string()),
        "data.parse.github-repo" => Some("github_repos_v1".to_string()),
        _ => None,
    }
}
