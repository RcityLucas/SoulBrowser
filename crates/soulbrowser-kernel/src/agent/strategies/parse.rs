use agent_core::plan::{AgentTool, AgentToolKind};
use agent_core::{requires_weather_pipeline, WaitMode};
use serde_json::json;
use std::collections::HashMap;

use super::{
    latest_observation_step, stage_overlay, StageStrategy, StrategyApplication, StrategyInput,
    StrategyStep,
};

#[derive(Debug, Default)]
pub struct GenericParseStrategy;

impl GenericParseStrategy {
    pub fn new() -> Self {
        Self::default()
    }
}

impl StageStrategy for GenericParseStrategy {
    fn id(&self) -> &'static str {
        "generic_parser"
    }

    fn stage(&self) -> agent_core::planner::PlanStageKind {
        agent_core::planner::PlanStageKind::Parse
    }

    fn apply(&self, input: &StrategyInput<'_>) -> Option<StrategyApplication> {
        let Some((_, observation_id)) = latest_observation_step(input.plan) else {
            return None;
        };
        let tool = AgentTool {
            kind: AgentToolKind::Custom {
                name: "data.parse.generic".to_string(),
                payload: json!({
                    "source_step_id": observation_id,
                    "schema": "generic_observation_v1",
                    "title": "Auto parse observation",
                    "detail": "Stage strategy generic parser",
                }),
            },
            wait: WaitMode::None,
            timeout_ms: Some(5_000),
        };
        let step = StrategyStep::new("解析采集数据", tool);
        Some(StrategyApplication {
            steps: vec![step],
            note: Some("自动追加 data.parse.generic".to_string()),
            overlay: Some(stage_overlay(
                agent_core::planner::PlanStageKind::Parse,
                self.id(),
                "applied",
                "🧠 追加通用解析",
            )),
            vendor_context: HashMap::new(),
        })
    }
}

#[derive(Debug, Default)]
pub struct WeatherParseStrategy;

impl WeatherParseStrategy {
    pub fn new() -> Self {
        Self::default()
    }
}

impl StageStrategy for WeatherParseStrategy {
    fn id(&self) -> &'static str {
        "weather_parser"
    }

    fn stage(&self) -> agent_core::planner::PlanStageKind {
        agent_core::planner::PlanStageKind::Parse
    }

    fn apply(&self, input: &StrategyInput<'_>) -> Option<StrategyApplication> {
        if !requires_weather_pipeline(input.request) {
            return None;
        }
        let Some((_, observation_id)) = latest_observation_step(input.plan) else {
            return None;
        };
        let tool = AgentTool {
            kind: AgentToolKind::Custom {
                name: "data.parse.weather".to_string(),
                payload: json!({
                    "source_step_id": observation_id,
                    "title": "Weather parser",
                    "detail": "Auto weather parser",
                }),
            },
            wait: WaitMode::Idle,
            timeout_ms: Some(8_000),
        };
        let step = StrategyStep::new("解析天气数据", tool);
        Some(StrategyApplication {
            steps: vec![step],
            note: Some("自动接入天气解析".to_string()),
            overlay: Some(stage_overlay(
                agent_core::planner::PlanStageKind::Parse,
                self.id(),
                "applied",
                "🌤️ 自动插入天气解析",
            )),
            vendor_context: HashMap::new(),
        })
    }
}

#[derive(Debug, Default)]
pub struct LlmSummaryStrategy;

impl LlmSummaryStrategy {
    pub fn new() -> Self {
        Self::default()
    }
}

impl StageStrategy for LlmSummaryStrategy {
    fn id(&self) -> &'static str {
        "llm_summary"
    }

    fn stage(&self) -> agent_core::planner::PlanStageKind {
        agent_core::planner::PlanStageKind::Parse
    }

    fn apply(&self, input: &StrategyInput<'_>) -> Option<StrategyApplication> {
        let Some((_, observation_id)) = latest_observation_step(input.plan) else {
            return None;
        };
        let summary = input
            .request
            .intent
            .primary_goal
            .clone()
            .unwrap_or_else(|| input.request.goal.clone());
        let parse_tool = AgentTool {
            kind: AgentToolKind::Custom {
                name: "data.parse.generic".to_string(),
                payload: json!({
                    "source_step_id": observation_id,
                    "schema": "generic_observation_v1",
                    "title": "LLM summary parser",
                    "detail": "Auto summary parse",
                }),
            },
            wait: WaitMode::None,
            timeout_ms: Some(4_000),
        };
        let note_tool = AgentTool {
            kind: AgentToolKind::Custom {
                name: "agent.note".to_string(),
                payload: json!({
                    "title": "自动总结",
                    "detail": summary,
                }),
            },
            wait: WaitMode::None,
            timeout_ms: Some(2_000),
        };
        let parse_step = StrategyStep::new("生成总结解析", parse_tool);
        let note_step = StrategyStep::new("总结当前页面", note_tool);
        Some(StrategyApplication {
            steps: vec![parse_step, note_step],
            note: Some("LLM summary fallback inserted".to_string()),
            overlay: Some(stage_overlay(
                agent_core::planner::PlanStageKind::Parse,
                self.id(),
                "applied",
                "🧠 使用 LLM 总结",
            )),
            vendor_context: HashMap::new(),
        })
    }
}
