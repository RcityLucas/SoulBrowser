use crate::model::{AgentIntentKind, AgentRequest};
use crate::plan::{AgentPlan, AgentPlanStep, AgentToolKind, AgentValidation, AgentWaitCondition};
use crate::weather::first_weather_subject;
use thiserror::Error;

const OBSERVATION_TOOLS: &[&str] = &["data.extract-site", "page.observe"];
const PARSE_TOOLS: &[&str] = &[
    "data.parse.generic",
    "data.parse.market_info",
    "data.parse.news_brief",
    "data.parse.weather",
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
    "data.extract-site, data.parse.generic, data.parse.market_info, data.parse.news_brief, data.parse.weather, data.parse.twitter-feed, data.parse.facebook-feed, data.parse.hackernews-feed, data.parse.linkedin-profile, data.parse.github-repo, data.deliver.structured, agent.note, plugin.*, mock.llm.plan";
const RESULT_KEYWORDS: &[&str] = &["查看", "获取", "告诉", "结果", "weather", "天气"];

#[derive(Debug, Error, Clone)]
pub enum PlanValidationIssue {
    #[error("{0}")]
    Composite(String),
    #[error("step '{step_title}' invoking data.deliver.structured must set payload.schema")]
    MissingDeliverSchema { step_id: String, step_title: String },
    #[error(
        "step '{step_title}' invoking data.deliver.structured must set payload.artifact_label"
    )]
    MissingDeliverArtifactLabel { step_id: String, step_title: String },
    #[error("step '{step_title}' invoking data.deliver.structured must set payload.filename")]
    MissingDeliverFilename { step_id: String, step_title: String },
    #[error(
        "step '{step_title}' invoking data.deliver.structured must set payload.source_step_id"
    )]
    MissingDeliverSourceStep { step_id: String, step_title: String },
    #[error(
        "step '{step_title}' invoking data.deliver.structured references unknown source_step_id '{source_step_id}'"
    )]
    DeliverSourceMissing {
        step_id: String,
        step_title: String,
        source_step_id: String,
    },
    #[error(
        "step '{step_title}' invoking data.deliver.structured must reference a parse step, but '{source_step_id}' is not a parser"
    )]
    DeliverSourceNotParse {
        step_id: String,
        step_title: String,
        source_step_id: String,
    },
    #[error(
        "step '{step_title}' invoking data.deliver.structured must reference an earlier parse step, but '{source_step_id}' appears later"
    )]
    DeliverSourceNotPrior {
        step_id: String,
        step_title: String,
        source_step_id: String,
    },
}

impl PlanValidationIssue {
    pub fn should_trigger_replan(&self) -> bool {
        matches!(self, PlanValidationIssue::MissingDeliverSchema { .. })
    }

    pub fn telemetry_label(&self) -> &'static str {
        match self {
            PlanValidationIssue::MissingDeliverSchema { .. } => "deliver_missing_schema",
            PlanValidationIssue::MissingDeliverArtifactLabel { .. } => {
                "deliver_missing_artifact_label"
            }
            PlanValidationIssue::MissingDeliverFilename { .. } => "deliver_missing_filename",
            PlanValidationIssue::MissingDeliverSourceStep { .. } => "deliver_missing_source_step",
            PlanValidationIssue::DeliverSourceMissing { .. } => "deliver_source_missing",
            PlanValidationIssue::DeliverSourceNotParse { .. } => "deliver_source_not_parse",
            PlanValidationIssue::DeliverSourceNotPrior { .. } => "deliver_source_not_prior",
            PlanValidationIssue::Composite(_) => "composite_violation",
        }
    }
}

#[derive(Debug, Clone)]
pub struct PlanValidator {
    strict: bool,
}

impl Default for PlanValidator {
    fn default() -> Self {
        Self { strict: false }
    }
}

impl PlanValidator {
    pub fn new(strict: bool) -> Self {
        Self { strict }
    }

    pub fn strict() -> Self {
        Self { strict: true }
    }

    pub fn validate(
        &self,
        plan: &AgentPlan,
        request: &AgentRequest,
    ) -> Result<(), PlanValidationIssue> {
        let mut issues = Vec::new();
        if let Some(err) = missing_github_username(plan) {
            issues.push(err);
        }

        if let Some(err) = navigation_missing_url(plan) {
            issues.push(err);
        }

        if let Some(err) = deliver_payload_issue(plan) {
            return Err(err);
        }

        if let Some(err) = missing_click_validations(plan) {
            issues.push(err);
        }

        if self.strict {
            self.collect_strict_requirements(plan, request, &mut issues);
        }

        if issues.is_empty() {
            Ok(())
        } else {
            Err(PlanValidationIssue::Composite(issues.join(" | ")))
        }
    }

    fn collect_strict_requirements(
        &self,
        plan: &AgentPlan,
        request: &AgentRequest,
        issues: &mut Vec<String>,
    ) {
        if plan_contains_plugin_tool(plan) {
            issues.push(
                "strict validation forbids plugin.* shims; planner must emit supported tools"
                    .to_string(),
            );
        }

        if !request.intent.target_sites.is_empty()
            && !targets_expected_site(plan, &request.intent.target_sites)
        {
            issues.push(format!(
                "plan must navigate to one of the preferred sites: {}",
                request.intent.target_sites.join(", ")
            ));
        }

        if let Some(err) = first_unsupported_custom_tool(plan) {
            issues.push(err);
        }

        if !request.intent.required_outputs.is_empty() && !plan_has_deliver_step(plan) {
            issues.push(
                "structured outputs requested but plan lacks data.deliver.structured".to_string(),
            );
        }

        if plan_has_dom_parser(plan) && !plan_has_observation(plan) {
            issues.push("DOM parsers require a prior data.extract-site observation".to_string());
        }

        if matches!(request.intent.intent_kind, AgentIntentKind::Informational) {
            if !plan_has_parse_step(plan) || !plan_has_user_result(plan) {
                issues.push(
                    "informational intents must parse data and surface a user-facing result"
                        .to_string(),
                );
            }
        }

        if requires_user_facing_result(request) && !plan_has_user_result(plan) {
            issues.push(
                "request expects a user-facing answer (agent.note or deliver step is required)"
                    .to_string(),
            );
        }

        if requires_weather_pipeline(request) && !plan_has_weather_pipeline(plan) {
            issues.push(
                "weather tasks must include data.parse.weather and structured delivery".to_string(),
            );
        }
    }
}

fn plan_has_observation(plan: &AgentPlan) -> bool {
    plan.steps.iter().any(|step| match &step.tool.kind {
        AgentToolKind::Custom { name, .. } => is_observation_tool(&name.to_ascii_lowercase()),
        _ => false,
    })
}

fn plan_has_parse_step(plan: &AgentPlan) -> bool {
    plan.steps.iter().any(|step| match &step.tool.kind {
        AgentToolKind::Custom { name, .. } => is_parse_tool(&name.to_ascii_lowercase()),
        _ => false,
    })
}

fn plan_has_dom_parser(plan: &AgentPlan) -> bool {
    plan.steps.iter().any(|step| match &step.tool.kind {
        AgentToolKind::Custom { name, .. } => is_dom_parser(name),
        _ => false,
    })
}

fn is_dom_parser(name: &str) -> bool {
    let canonical = name.trim().to_ascii_lowercase();
    matches!(
        canonical.as_str(),
        "data.parse.generic"
            | "data.parse.market_info"
            | "data.parse.news_brief"
            | "data.parse.weather"
            | "data.parse.twitter-feed"
            | "data.parse.facebook-feed"
            | "data.parse.hackernews-feed"
            | "data.parse.linkedin-profile"
    )
}

fn plan_has_deliver_step(plan: &AgentPlan) -> bool {
    plan.steps.iter().any(|step| match &step.tool.kind {
        AgentToolKind::Custom { name, .. } => is_deliver_tool(name),
        _ => false,
    })
}

fn plan_has_note_step(plan: &AgentPlan) -> bool {
    plan.steps.iter().any(|step| match &step.tool.kind {
        AgentToolKind::Custom { name, .. } => is_note_tool(name),
        _ => false,
    })
}

fn plan_has_user_result(plan: &AgentPlan) -> bool {
    plan_has_deliver_step(plan) || plan_has_note_step(plan)
}

fn plan_has_weather_parser(plan: &AgentPlan) -> bool {
    plan.steps.iter().any(|step| match &step.tool.kind {
        AgentToolKind::Custom { name, .. } => name.eq_ignore_ascii_case("data.parse.weather"),
        _ => false,
    })
}

fn plan_has_weather_deliver(plan: &AgentPlan) -> bool {
    plan.steps.iter().any(|step| match &step.tool.kind {
        AgentToolKind::Custom { name, payload }
            if is_deliver_tool(name) && payload_contains_weather_schema(payload) =>
        {
            true
        }
        _ => false,
    })
}

fn payload_contains_weather_schema(payload: &serde_json::Value) -> bool {
    payload
        .as_object()
        .and_then(|map| map.get("schema"))
        .and_then(|value| value.as_str())
        .map(|schema| {
            schema
                .trim()
                .trim_end_matches(".json")
                .eq_ignore_ascii_case("weather_report_v1")
        })
        .unwrap_or(false)
}

fn plan_has_weather_pipeline(plan: &AgentPlan) -> bool {
    plan_has_weather_parser(plan) && plan_has_weather_deliver(plan)
}

fn plan_contains_plugin_tool(plan: &AgentPlan) -> bool {
    plan.steps.iter().any(|step| match &step.tool.kind {
        AgentToolKind::Custom { name, .. } => name.starts_with("plugin."),
        _ => false,
    })
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

pub fn is_allowed_custom_tool(name: &str) -> bool {
    let trimmed = name.trim();
    let canonical = trimmed.to_ascii_lowercase();
    is_observation_tool(&canonical)
        || is_parse_tool(&canonical)
        || is_deliver_tool(&canonical)
        || is_note_tool(&canonical)
        || canonical.starts_with("plugin.")
        || canonical == "weather.search"
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

pub fn requires_user_facing_result(request: &AgentRequest) -> bool {
    contains_result_keywords(&request.goal)
        || request
            .intent
            .primary_goal
            .as_deref()
            .map(contains_result_keywords)
            .unwrap_or(false)
}

fn contains_result_keywords(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    RESULT_KEYWORDS.iter().any(|keyword| {
        let trimmed = keyword.trim();
        !trimmed.is_empty()
            && (text.contains(trimmed) || lower.contains(&trimmed.to_ascii_lowercase()))
    })
}

pub fn requires_weather_pipeline(request: &AgentRequest) -> bool {
    let mut sources = Vec::new();
    if let Some(primary) = request.intent.primary_goal.as_deref() {
        sources.push(primary);
    }
    sources.push(request.goal.as_str());
    first_weather_subject(sources.iter().copied()).is_some()
        || request
            .intent
            .required_outputs
            .iter()
            .any(|output| schema_matches_weather(&output.schema))
}

fn schema_matches_weather(schema: &str) -> bool {
    let normalized = schema.trim().trim_end_matches(".json").to_ascii_lowercase();
    normalized == "weather_report_v1"
}

fn missing_click_validations(plan: &AgentPlan) -> Option<String> {
    plan.steps.iter().find_map(|step| match step.tool.kind {
        AgentToolKind::Click { .. } if !has_required_click_validation(step) => Some(format!(
            "click step '{}' must include wait_for url contains or DOM validation",
            step.title
        )),
        _ => None,
    })
}

fn has_required_click_validation(step: &AgentPlanStep) -> bool {
    step.validations.iter().any(validation_covers_navigation)
}

fn validation_covers_navigation(validation: &AgentValidation) -> bool {
    matches!(
        validation.condition,
        AgentWaitCondition::UrlMatches(_)
            | AgentWaitCondition::UrlEquals(_)
            | AgentWaitCondition::TitleMatches(_)
            | AgentWaitCondition::ElementVisible(_)
            | AgentWaitCondition::ElementHidden(_)
    )
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

fn navigation_missing_url(plan: &AgentPlan) -> Option<String> {
    plan.steps.iter().find_map(|step| {
        if let AgentToolKind::Navigate { url } = &step.tool.kind {
            if url.trim().is_empty() {
                return Some(format!(
                    "step '{}' invoking navigate must specify a non-empty url",
                    step.title
                ));
            }
        }
        None
    })
}

fn deliver_payload_issue(plan: &AgentPlan) -> Option<PlanValidationIssue> {
    plan.steps.iter().enumerate().find_map(|(idx, step)| {
        if let AgentToolKind::Custom { name, payload } = &step.tool.kind {
            if is_deliver_tool(name) {
                let step_id = step.id.clone();
                let step_title = step.title.clone();

                if payload_string(payload, "schema").is_none() {
                    return Some(PlanValidationIssue::MissingDeliverSchema {
                        step_id,
                        step_title,
                    });
                }

                if payload_string(payload, "artifact_label").is_none() {
                    return Some(PlanValidationIssue::MissingDeliverArtifactLabel {
                        step_id,
                        step_title,
                    });
                }

                if payload_string(payload, "filename").is_none() {
                    return Some(PlanValidationIssue::MissingDeliverFilename {
                        step_id,
                        step_title,
                    });
                }

                let source_step_id = match payload_string(payload, "source_step_id") {
                    Some(value) => value,
                    None => {
                        return Some(PlanValidationIssue::MissingDeliverSourceStep {
                            step_id,
                            step_title,
                        })
                    }
                };

                let Some(source_index) = plan
                    .steps
                    .iter()
                    .position(|candidate| candidate.id == source_step_id)
                else {
                    return Some(PlanValidationIssue::DeliverSourceMissing {
                        step_id,
                        step_title,
                        source_step_id,
                    });
                };

                if source_index >= idx {
                    return Some(PlanValidationIssue::DeliverSourceNotPrior {
                        step_id,
                        step_title,
                        source_step_id,
                    });
                }

                let source_step = &plan.steps[source_index];
                let is_parse_source = matches!(
                    &source_step.tool.kind,
                    AgentToolKind::Custom { name, .. }
                        if is_parse_tool(name) || name.starts_with("plugin.")
                );
                if !is_parse_source {
                    return Some(PlanValidationIssue::DeliverSourceNotParse {
                        step_id,
                        step_title,
                        source_step_id,
                    });
                }
            }
        }
        None
    })
}

fn payload_string(payload: &serde_json::Value, key: &str) -> Option<String> {
    payload
        .get(key)
        .and_then(|value| value.as_str())
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}
