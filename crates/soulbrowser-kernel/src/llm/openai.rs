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
pub struct OpenAiConfig {
    pub api_key: String,
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
                    content: self
                        .prompt
                        .build_user_prompt(request, previous_plan, failure_summary),
                },
            ],
        };

        let url = format!(
            "{}/chat/completions",
            self.config.api_base.trim_end_matches('/')
        );

        let response = self
            .client
            .post(url)
            .bearer_auth(&self.config.api_key)
            .json(&body)
            .send()
            .await
            .map_err(|err| AgentError::invalid_request(format!("openai request failed: {err}")))?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response
                .text()
                .await
                .unwrap_or_else(|_| "<response unavailable>".to_string());
            return Err(AgentError::invalid_request(format!(
                "openai returned {}: {}",
                status, text
            )));
        }

        let response: ChatCompletionResponse = response.json().await.map_err(|err| {
            AgentError::invalid_request(format!("openai response invalid: {err}"))
        })?;

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

        plan_from_json_payload(request, payload)
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
