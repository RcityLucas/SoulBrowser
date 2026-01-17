use crate::llm::agent_loop_prompt::{
    build_system_prompt as build_agent_loop_system_prompt,
    build_user_message as build_agent_loop_user_message,
    parse_agent_output as parse_agent_loop_output,
};
use crate::llm::prompt::PromptBuilder;
use crate::llm::schema::{plan_from_json_payload, LlmJsonPlan};
use crate::llm::utils::extract_json_object;
use agent_core::AgentError;
use agent_core::{
    AgentHistoryEntry, AgentOutput, AgentPlan, AgentRequest, BrowserStateSummary, LlmProvider,
    PlannerOutcome,
};
use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value as JsonValue};
use std::time::Duration;
use tracing::warn;

#[derive(Debug, Clone)]
pub struct OpenAiConfig {
    pub api_keys: Vec<String>,
    pub model: String,
    pub api_base: String,
    pub temperature: f32,
    pub timeout: Duration,
}

pub struct OpenAiLlmProvider {
    client: Client,
    prompt: PromptBuilder,
    config: OpenAiConfig,
}

impl OpenAiLlmProvider {
    pub fn new(config: OpenAiConfig) -> Result<Self, AgentError> {
        if config.api_keys.is_empty() {
            return Err(AgentError::invalid_request(
                "missing OpenAI API key for planner",
            ));
        }
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
        let url = format!(
            "{}/chat/completions",
            self.config.api_base.trim_end_matches('/')
        );

        let mut last_error: Option<AgentError> = None;
        for (index, key) in self.config.api_keys.iter().enumerate() {
            let body = ChatCompletionRequest {
                model: self.config.model.clone(),
                temperature: self.config.temperature,
                response_format: ResponseFormat {
                    r#type: "json_object".to_string(),
                },
                messages: vec![
                    ChatMessage {
                        role: "system".to_string(),
                        content: self.prompt.system_prompt().to_string(),
                    },
                    ChatMessage {
                        role: "user".to_string(),
                        content: self.prompt.build_user_prompt(
                            request,
                            previous_plan,
                            failure_summary,
                        ),
                    },
                ],
            };

            let response = self
                .client
                .post(&url)
                .bearer_auth(key)
                .json(&body)
                .send()
                .await;

            let response = match response {
                Ok(resp) => resp,
                Err(err) => {
                    last_error = Some(AgentError::invalid_request(format!(
                        "openai request failed: {err}"
                    )));
                    continue;
                }
            };

            if !response.status().is_success() {
                let status = response.status();
                let text = response
                    .text()
                    .await
                    .unwrap_or_else(|_| "<response unavailable>".to_string());
                if status.as_u16() == 429 && index + 1 < self.config.api_keys.len() {
                    let friendly = openai_rate_limit_message(&text);
                    warn!(
                        target: "openai",
                        message = %friendly,
                        raw = %text,
                        attempt = index + 1,
                        remaining = self.config.api_keys.len() - index - 1,
                        "OpenAI rate limited plan request; switching API key"
                    );
                    last_error = Some(AgentError::invalid_request(friendly));
                    continue;
                }
                let err =
                    AgentError::invalid_request(format!("openai returned {}: {}", status, text));
                return Err(err);
            }

            let response: ChatCompletionResponse = response.json().await.map_err(|err| {
                AgentError::invalid_request(format!("openai response invalid: {err}"))
            })?;

            let usage = response.usage.clone();
            let content = response
                .choices
                .first()
                .and_then(|choice| choice.message.content.as_text())
                .ok_or_else(|| AgentError::invalid_request("openai response missing content"))?;

            let json_string = extract_json_object(&content)
                .ok_or_else(|| AgentError::invalid_request("openai response missing JSON plan"))?;

            let payload: LlmJsonPlan = serde_json::from_str(&json_string).map_err(|err| {
                AgentError::invalid_request(format!("failed to parse LLM plan JSON: {err}"))
            })?;
            let mut outcome = plan_from_json_payload(request, payload)?;
            if let Some(usage) = usage {
                annotate_plan_with_usage(&mut outcome.plan, &usage);
            }
            return Ok(outcome);
        }

        Err(last_error.unwrap_or_else(|| {
            AgentError::invalid_request("OpenAI request exhausted all API keys")
        }))
    }
}

#[async_trait]
impl LlmProvider for OpenAiLlmProvider {
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

    async fn decide(
        &self,
        request: &AgentRequest,
        state: &BrowserStateSummary,
        history: &[AgentHistoryEntry],
    ) -> Result<AgentOutput, AgentError> {
        let url = format!(
            "{}/chat/completions",
            self.config.api_base.trim_end_matches('/')
        );

        let mut last_error: Option<AgentError> = None;
        for (index, key) in self.config.api_keys.iter().enumerate() {
            let body = ChatCompletionRequest {
                model: self.config.model.clone(),
                temperature: self.config.temperature,
                response_format: ResponseFormat {
                    r#type: "json_object".to_string(),
                },
                messages: vec![
                    ChatMessage {
                        role: "system".to_string(),
                        content: build_agent_loop_system_prompt(state),
                    },
                    ChatMessage {
                        role: "user".to_string(),
                        content: build_agent_loop_user_message(request, state, history),
                    },
                ],
            };

            let response = self
                .client
                .post(&url)
                .bearer_auth(key)
                .json(&body)
                .send()
                .await;

            let response = match response {
                Ok(resp) => resp,
                Err(err) => {
                    last_error = Some(AgentError::invalid_request(format!(
                        "openai request failed: {err}"
                    )));
                    continue;
                }
            };

            if !response.status().is_success() {
                let status = response.status();
                let text = response
                    .text()
                    .await
                    .unwrap_or_else(|_| "<response unavailable>".to_string());
                if status.as_u16() == 429 && index + 1 < self.config.api_keys.len() {
                    let friendly = openai_rate_limit_message(&text);
                    warn!(
                        target: "openai",
                        message = %friendly,
                        raw = %text,
                        attempt = index + 1,
                        remaining = self.config.api_keys.len() - index - 1,
                        "OpenAI rate limited agent loop decide; switching API key"
                    );
                    last_error = Some(AgentError::invalid_request(friendly));
                    continue;
                }
                let err =
                    AgentError::invalid_request(format!("openai returned {}: {}", status, text));
                return Err(err);
            }

            let response: ChatCompletionResponse = response.json().await.map_err(|err| {
                AgentError::invalid_request(format!("openai response invalid: {err}"))
            })?;

            let content = response
                .choices
                .first()
                .and_then(|choice| choice.message.content.as_text())
                .ok_or_else(|| AgentError::invalid_request("openai response missing content"))?;

            return parse_agent_loop_output(&content);
        }

        Err(last_error.unwrap_or_else(|| {
            AgentError::invalid_request("OpenAI request exhausted all API keys")
        }))
    }
}

#[derive(Debug, Serialize)]
struct ChatCompletionRequest {
    model: String,
    temperature: f32,
    response_format: ResponseFormat,
    messages: Vec<ChatMessage>,
}

#[derive(Debug, Serialize)]
struct ResponseFormat {
    r#type: String,
}

#[derive(Debug, Serialize)]
struct ChatMessage {
    role: String,
    content: String,
}

#[derive(Debug, Deserialize)]
struct ChatCompletionResponse {
    choices: Vec<ChatCompletionChoice>,
    #[serde(default)]
    usage: Option<ChatCompletionUsage>,
}

#[derive(Debug, Deserialize)]
struct ChatCompletionChoice {
    message: ChatCompletionMessage,
}

#[derive(Debug, Deserialize)]
struct ChatCompletionMessage {
    content: ChatCompletionContent,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum ChatCompletionContent {
    Text(String),
    Parts(Vec<ChatCompletionPart>),
}

impl ChatCompletionContent {
    fn as_text(&self) -> Option<String> {
        match self {
            ChatCompletionContent::Text(value) => Some(value.clone()),
            ChatCompletionContent::Parts(parts) => {
                let text = parts
                    .iter()
                    .filter_map(|part| part.text.as_ref())
                    .cloned()
                    .collect::<Vec<_>>()
                    .join("\n");
                if text.is_empty() {
                    None
                } else {
                    Some(text)
                }
            }
        }
    }
}

#[derive(Debug, Deserialize)]
struct ChatCompletionPart {
    #[serde(default)]
    text: Option<String>,
}

fn annotate_plan_with_usage(plan: &mut AgentPlan, usage: &ChatCompletionUsage) {
    for step in plan.steps.iter_mut() {
        let entry = step
            .metadata
            .entry("agent_state".to_string())
            .or_insert_with(|| JsonValue::Object(Map::new()));
        if let JsonValue::Object(ref mut map) = entry {
            map.insert(
                "llm_input_tokens".to_string(),
                JsonValue::from(usage.prompt_tokens),
            );
            map.insert(
                "llm_output_tokens".to_string(),
                JsonValue::from(usage.completion_tokens),
            );
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
struct ChatCompletionUsage {
    #[serde(default)]
    prompt_tokens: u64,
    #[serde(default)]
    completion_tokens: u64,
}

#[derive(Debug, Deserialize)]
struct OpenAiErrorEnvelope {
    error: OpenAiErrorMessage,
}

#[derive(Debug, Deserialize)]
struct OpenAiErrorMessage {
    message: Option<String>,
    #[serde(default)]
    _code: Option<String>,
    #[serde(default)]
    _type: Option<String>,
}

fn openai_rate_limit_message(raw: &str) -> String {
    if let Ok(envelope) = serde_json::from_str::<OpenAiErrorEnvelope>(raw) {
        if let Some(message) = envelope.error.message {
            return format!(
                "OpenAI rate limit exceeded: {}. Please retry later or configure a higher tier.",
                message.trim()
            );
        }
    }
    "OpenAI rate limit exceeded; please retry later or reduce usage.".to_string()
}
