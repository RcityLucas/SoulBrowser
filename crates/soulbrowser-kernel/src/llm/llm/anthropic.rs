use crate::llm::prompt::PromptBuilder;
use crate::llm::schema::{plan_from_json_payload, LlmJsonPlan};
use crate::llm::utils::extract_json_object;
use agent_core::AgentError;
use agent_core::{AgentPlan, AgentRequest, LlmProvider, PlannerOutcome};
use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct ClaudeConfig {
    pub api_key: String,
    pub model: String,
    pub api_base: String,
    pub temperature: f32,
    pub max_tokens: u32,
    pub timeout: Duration,
}

pub struct ClaudeLlmProvider {
    client: Client,
    prompt: PromptBuilder,
    config: ClaudeConfig,
}

impl ClaudeLlmProvider {
    pub fn new(config: ClaudeConfig) -> Result<Self, AgentError> {
        let client = Client::builder()
            .timeout(config.timeout)
            .build()
            .map_err(|err| {
                AgentError::invalid_request(format!("failed to build HTTP client: {err}"))
            })?;
        Ok(Self {
            client,
            prompt: PromptBuilder::new(),
            config,
        })
    }

    async fn invoke(
        &self,
        request: &AgentRequest,
        previous_plan: Option<&AgentPlan>,
        failure_summary: Option<&str>,
    ) -> Result<PlannerOutcome, AgentError> {
        let body = ClaudeRequest {
            model: self.config.model.clone(),
            temperature: self.config.temperature,
            max_tokens: self.config.max_tokens,
            system: self.prompt.system_prompt().to_string(),
            messages: vec![ClaudeMessage {
                role: "user".to_string(),
                content: vec![ClaudeContent {
                    _type: "text".to_string(),
                    text: self
                        .prompt
                        .build_user_prompt(request, previous_plan, failure_summary),
                }],
            }],
        };

        let url = format!("{}/messages", self.config.api_base.trim_end_matches('/'));

        let response = self
            .client
            .post(url)
            .header("x-api-key", &self.config.api_key)
            .header("anthropic-version", "2023-06-01")
            .json(&body)
            .send()
            .await
            .map_err(|err| AgentError::invalid_request(format!("claude request failed: {err}")))?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response
                .text()
                .await
                .unwrap_or_else(|_| "<response unavailable>".to_string());
            return Err(AgentError::invalid_request(format!(
                "claude returned {}: {}",
                status, text
            )));
        }

        let response: ClaudeResponse = response.json().await.map_err(|err| {
            AgentError::invalid_request(format!("claude response invalid: {err}"))
        })?;

        let content = response
            .content
            .iter()
            .filter_map(|part| part.text.as_ref())
            .cloned()
            .collect::<Vec<_>>()
            .join("\n");

        if content.is_empty() {
            return Err(AgentError::invalid_request(
                "claude response missing content",
            ));
        }

        let json_string = extract_json_object(&content)
            .ok_or_else(|| AgentError::invalid_request("claude response missing JSON plan"))?;

        let payload: LlmJsonPlan = serde_json::from_str(&json_string).map_err(|err| {
            AgentError::invalid_request(format!("failed to parse LLM plan JSON: {err}"))
        })?;

        plan_from_json_payload(request, payload)
    }
}

#[async_trait]
impl LlmProvider for ClaudeLlmProvider {
    async fn plan(&self, request: &AgentRequest) -> Result<PlannerOutcome, AgentError> {
        self.invoke(request, None, None).await
    }

    async fn replan(
        &self,
        request: &AgentRequest,
        previous_plan: &AgentPlan,
        error_summary: &str,
    ) -> Result<PlannerOutcome, AgentError> {
        self.invoke(request, Some(previous_plan), Some(error_summary))
            .await
    }
}

#[derive(Debug, Serialize)]
struct ClaudeRequest {
    model: String,
    temperature: f32,
    #[serde(rename = "max_tokens")]
    max_tokens: u32,
    system: String,
    messages: Vec<ClaudeMessage>,
}

#[derive(Debug, Serialize)]
struct ClaudeMessage {
    role: String,
    content: Vec<ClaudeContent>,
}

#[derive(Debug, Serialize)]
struct ClaudeContent {
    #[serde(rename = "type")]
    _type: String,
    text: String,
}

#[derive(Debug, Deserialize)]
struct ClaudeResponse {
    content: Vec<ClaudeResponseContent>,
}

#[derive(Debug, Deserialize)]
struct ClaudeResponseContent {
    #[serde(rename = "type")]
    _type: String,
    #[serde(default)]
    text: Option<String>,
}
