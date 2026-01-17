use crate::plan::{AgentPlan, AgentPlanStep, AgentToolKind};
use serde::{Deserialize, Serialize};

/// Canonical stage names that high level plans travel through.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanStageKind {
    Navigate,
    Observe,
    Validate,
    Act,
    Evaluate,
    Parse,
    Deliver,
}

impl PlanStageKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            PlanStageKind::Navigate => "navigate",
            PlanStageKind::Observe => "observe",
            PlanStageKind::Validate => "validate",
            PlanStageKind::Act => "act",
            PlanStageKind::Evaluate => "evaluate",
            PlanStageKind::Parse => "parse",
            PlanStageKind::Deliver => "deliver",
        }
    }

    pub fn from_str(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "navigate" => Some(PlanStageKind::Navigate),
            "observe" => Some(PlanStageKind::Observe),
            "validate" => Some(PlanStageKind::Validate),
            "act" => Some(PlanStageKind::Act),
            "evaluate" => Some(PlanStageKind::Evaluate),
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
        AgentToolKind::Done { .. } => vec![PlanStageKind::Deliver],
    }
}

fn classify_custom_tool(name: &str) -> Vec<PlanStageKind> {
    let lowered = name.trim().to_ascii_lowercase();
    if matches!(
        lowered.as_str(),
        "data.extract-site" | "page.observe" | "market.quote.fetch"
    ) {
        vec![PlanStageKind::Observe]
    } else if lowered == "data.validate-target" || lowered == "data.validate.metal_price" {
        vec![PlanStageKind::Validate]
    } else if lowered == "weather.search" || lowered == "browser.search" {
        vec![PlanStageKind::Navigate]
    } else if lowered == "browser.close-modal" || lowered == "browser.send-esc" {
        vec![PlanStageKind::Act]
    } else if lowered == "agent.evaluate" {
        vec![PlanStageKind::Evaluate]
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
