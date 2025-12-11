use crate::errors::AgentError;
use crate::model::AgentRequest;
use crate::plan::{AgentPlan, AgentPlanStep, AgentToolKind};

const OBSERVATION_TOOLS: &[&str] = &["data.extract-site", "page.observe"];
const PARSE_TOOLS: &[&str] = &[
    "data.parse.generic",
    "data.parse.market_info",
    "data.parse.news_brief",
    "data.parse.twitter-feed",
    "data.parse.facebook-feed",
    "data.parse.hackernews-feed",
    "data.parse.linkedin-profile",
    "data.parse.github-repo",
];
const PARSE_TOOL_ALIASES: &[&str] = &[
    "parse",
    "github.extract-repo",
    "data.parse.github.extract-repo",
    "data.parse.twitter_feed",
    "data.parse.twitter.feed",
    "data.parse.facebook_feed",
    "data.parse.facebook.feed",
    "data.parse.hackernews_feed",
    "data.parse.hackernews.feed",
    "data.parse.linkedin_profile",
    "data.parse.linkedin.profile",
];
const DELIVER_TOOLS: &[&str] = &["data.deliver.structured"];
const NOTE_TOOLS: &[&str] = &["agent.note"];
const ALLOWED_CUSTOM_TOOL_HINT: &str =
    "data.extract-site, data.parse.generic, data.parse.market_info, data.parse.news_brief, data.parse.twitter-feed, data.parse.facebook-feed, data.parse.hackernews-feed, data.parse.linkedin-profile, data.parse.github-repo, data.deliver.structured, agent.note, plugin.*, mock.llm.plan";

#[derive(Debug, Default, Clone)]
pub struct PlanValidator;

impl PlanValidator {
    pub fn validate(&self, plan: &AgentPlan, request: &AgentRequest) -> Result<(), AgentError> {
        let mut issues = Vec::new();

        if !request.intent.target_sites.is_empty()
            && !targets_expected_site(plan, &request.intent.target_sites)
        {
            issues.push(format!(
                "plan must navigate to one of the preferred sites: {}",
                request.intent.target_sites.join(", ")
            ));
        }

        if !request.intent.required_outputs.is_empty() {
            if !has_observation_step(plan) {
                issues.push("structured outputs require at least one observation step".to_string());
            }
            if !self.enforces_stage_order(plan, true) {
                issues.push(
                    "plan must respect navigate -> observe -> act -> parse -> deliver ordering when emitting structured data"
                        .to_string(),
                );
            }
        }

        if let Some(err) = first_unsupported_custom_tool(plan) {
            issues.push(err);
        }

        if let Some(err) = missing_github_username(plan) {
            issues.push(err);
        }

        if issues.is_empty() {
            Ok(())
        } else {
            Err(AgentError::unsupported(issues.join(" | ")))
        }
    }

    fn enforces_stage_order(&self, plan: &AgentPlan, require_deliver: bool) -> bool {
        let stages = [
            PlanStage::Navigate,
            PlanStage::Observe,
            PlanStage::Act,
            PlanStage::Parse,
            PlanStage::Deliver,
        ];
        let mut cursor = 0usize;
        for stage in stages {
            if matches!(stage, PlanStage::Act) && !has_act_step(plan) {
                continue;
            }
            if matches!(stage, PlanStage::Parse) && !has_parse_step(plan) {
                continue;
            }
            if matches!(stage, PlanStage::Deliver) {
                let has_deliver = has_deliver_step(plan);
                if !require_deliver && !has_deliver {
                    break;
                }
                if !has_deliver {
                    return false;
                }
            }

            if let Some(idx) = find_stage_index(plan, stage, cursor) {
                cursor = idx;
                continue;
            }

            return false;
        }
        true
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PlanStage {
    Navigate,
    Observe,
    Act,
    Parse,
    Deliver,
}

fn find_stage_index(plan: &AgentPlan, stage: PlanStage, start: usize) -> Option<usize> {
    plan.steps
        .iter()
        .enumerate()
        .skip(start)
        .find(|(_, step)| classify_step(step).contains(&stage))
        .map(|(idx, _)| idx)
}

fn classify_step(step: &AgentPlanStep) -> Vec<PlanStage> {
    match &step.tool.kind {
        AgentToolKind::Navigate { .. } => vec![PlanStage::Navigate],
        AgentToolKind::Click { .. }
        | AgentToolKind::TypeText { .. }
        | AgentToolKind::Select { .. }
        | AgentToolKind::Scroll { .. }
        | AgentToolKind::Wait { .. } => vec![PlanStage::Act],
        AgentToolKind::Custom { name, .. } => {
            if is_observation_tool(name) {
                vec![PlanStage::Observe]
            } else if is_parse_tool(name) {
                if is_github_repo_tool(name) {
                    vec![PlanStage::Observe, PlanStage::Parse]
                } else {
                    vec![PlanStage::Parse]
                }
            } else if is_deliver_tool(name) {
                vec![PlanStage::Deliver]
            } else if is_note_tool(name) {
                vec![PlanStage::Deliver]
            } else {
                Vec::new()
            }
        }
    }
}

fn has_observation_step(plan: &AgentPlan) -> bool {
    plan.steps.iter().any(|step| {
        matches!(&step.tool.kind,
            AgentToolKind::Custom { name, .. } if is_observation_tool(name) || is_github_repo_tool(name))
    })
}

fn has_act_step(plan: &AgentPlan) -> bool {
    plan.steps
        .iter()
        .any(|step| classify_step(step).contains(&PlanStage::Act))
}

fn has_parse_step(plan: &AgentPlan) -> bool {
    plan.steps.iter().any(
        |step| matches!(&step.tool.kind, AgentToolKind::Custom { name, .. } if is_parse_tool(name)),
    )
}

fn has_deliver_step(plan: &AgentPlan) -> bool {
    plan.steps
        .iter()
        .any(|step| classify_step(step).contains(&PlanStage::Deliver))
}

fn targets_expected_site(plan: &AgentPlan, preferred_sites: &[String]) -> bool {
    if preferred_sites.is_empty() {
        return true;
    }
    plan.steps.iter().any(|step| match &step.tool.kind {
        AgentToolKind::Navigate { url } => preferred_sites.iter().any(|site| url.contains(site)),
        _ => false,
    })
}

fn first_unsupported_custom_tool(plan: &AgentPlan) -> Option<String> {
    plan.steps.iter().find_map(|step| match &step.tool.kind {
        AgentToolKind::Custom { name, .. } if !is_allowed_custom_tool(name) => Some(format!(
            "step '{}' uses unsupported custom tool '{}'. Allowed custom tools: {}",
            step.title, name, ALLOWED_CUSTOM_TOOL_HINT
        )),
        _ => None,
    })
}

fn is_allowed_custom_tool(name: &str) -> bool {
    let trimmed = name.trim();
    let canonical = trimmed.to_ascii_lowercase();
    is_observation_tool(&canonical)
        || is_parse_tool(&canonical)
        || is_deliver_tool(&canonical)
        || is_note_tool(&canonical)
        || canonical.starts_with("plugin.")
        || canonical == "mock.llm.plan"
}

fn is_observation_tool(name: &str) -> bool {
    OBSERVATION_TOOLS
        .iter()
        .any(|tool| tool.eq_ignore_ascii_case(name))
}

fn is_parse_tool(name: &str) -> bool {
    PARSE_TOOLS
        .iter()
        .chain(PARSE_TOOL_ALIASES.iter())
        .any(|tool| tool.eq_ignore_ascii_case(name))
}

fn is_github_repo_tool(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "data.parse.github-repo" | "github.extract-repo" | "data.parse.github.extract-repo"
    )
}

fn is_deliver_tool(name: &str) -> bool {
    DELIVER_TOOLS
        .iter()
        .any(|tool| tool.eq_ignore_ascii_case(name))
        || name.starts_with("data.deliver.")
}

fn is_note_tool(name: &str) -> bool {
    NOTE_TOOLS
        .iter()
        .any(|tool| tool.eq_ignore_ascii_case(name))
        || name.ends_with("note")
}

fn missing_github_username(plan: &AgentPlan) -> Option<String> {
    plan.steps.iter().find_map(|step| {
        if let AgentToolKind::Custom { name, payload } = &step.tool.kind {
            if is_github_repo_tool(name) {
                let username = payload
                    .get("username")
                    .and_then(|value| value.as_str())
                    .map(|s| s.trim())
                    .filter(|s| !s.is_empty());
                if username.is_none() {
                    return Some(format!(
                        "step '{}' invoking data.parse.github-repo must set payload.username (GitHub handle without '@')",
                        step.title
                    ));
                }
            }
        }
        None
    })
}
