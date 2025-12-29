use crate::plan::{AgentPlan, AgentPlanStep, AgentToolKind};
use serde::{Deserialize, Serialize};

/// Canonical stage names that high level plans travel through.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanStageKind {
    Navigate,
    Observe,
    Act,
    Parse,
    Deliver,
}

impl PlanStageKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            PlanStageKind::Navigate => "navigate",
            PlanStageKind::Observe => "observe",
            PlanStageKind::Act => "act",
            PlanStageKind::Parse => "parse",
            PlanStageKind::Deliver => "deliver",
        }
    }

    pub fn from_str(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "navigate" => Some(PlanStageKind::Navigate),
            "observe" => Some(PlanStageKind::Observe),
            "act" => Some(PlanStageKind::Act),
            "parse" => Some(PlanStageKind::Parse),
            "deliver" => Some(PlanStageKind::Deliver),
            _ => None,
        }
    }
}

/// Classify a plan step into one or more stage buckets.
pub fn classify_step(step: &AgentPlanStep) -> Vec<PlanStageKind> {
    match &step.tool.kind {
        AgentToolKind::Navigate { .. } => vec![PlanStageKind::Navigate],
        AgentToolKind::Click { .. }
        | AgentToolKind::TypeText { .. }
        | AgentToolKind::Select { .. }
        | AgentToolKind::Scroll { .. }
        | AgentToolKind::Wait { .. } => vec![PlanStageKind::Act],
        AgentToolKind::Custom { name, .. } => classify_custom_tool(name),
    }
}

fn classify_custom_tool(name: &str) -> Vec<PlanStageKind> {
    let lowered = name.trim().to_ascii_lowercase();
    if matches!(lowered.as_str(), "data.extract-site" | "page.observe") {
        vec![PlanStageKind::Observe]
    } else if lowered == "weather.search" {
        vec![PlanStageKind::Navigate]
    } else if is_parse_tool(&lowered) {
        if lowered == "data.parse.github-repo" {
            vec![PlanStageKind::Observe, PlanStageKind::Parse]
        } else {
            vec![PlanStageKind::Parse]
        }
    } else if lowered == "data.deliver.structured" || lowered == "agent.note" {
        vec![PlanStageKind::Deliver]
    } else {
        Vec::new()
    }
}

fn is_parse_tool(name: &str) -> bool {
    matches!(
        name,
        "data.parse.generic"
            | "data.parse.market_info"
            | "data.parse.news_brief"
            | "data.parse.weather"
            | "data.parse.twitter-feed"
            | "data.parse.facebook-feed"
            | "data.parse.hackernews-feed"
            | "data.parse.linkedin-profile"
            | "data.parse.github-repo"
            | "github.extract-repo"
            | "data.parse.github.extract-repo"
            | "parse"
    )
}

pub fn plan_contains_stage(plan: &AgentPlan, stage: PlanStageKind) -> bool {
    plan.steps
        .iter()
        .any(|step| classify_step(step).contains(&stage))
}

pub fn stage_index(plan: &AgentPlan, stage: PlanStageKind, start: usize) -> Option<usize> {
    plan.steps
        .iter()
        .enumerate()
        .skip(start)
        .find(|(_, step)| classify_step(step).contains(&stage))
        .map(|(idx, _)| idx)
}
