use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use soulbrowser_core_types::{PageId, SessionId, TaskId};
use std::collections::HashMap;

/// Role of a conversation turn exchanged with the agent.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ConversationRole {
    /// Human operator issuing requests.
    User,
    /// Agent assistant responding with plans or clarifications.
    Assistant,
    /// System generated instructions or policies.
    System,
}

/// A single conversational message that provides context for planning.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationTurn {
    pub role: ConversationRole,
    pub message: String,
    pub timestamp: DateTime<Utc>,
}

impl ConversationTurn {
    pub fn new(role: ConversationRole, message: impl Into<String>) -> Self {
        Self {
            role,
            message: message.into(),
            timestamp: Utc::now(),
        }
    }
}

/// Snapshot of execution context provided to the agent planner.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AgentContext {
    /// Active session tracked by L0-L1 layers.
    pub session: Option<SessionId>,
    /// Currently focused page identifier.
    pub page: Option<PageId>,
    /// Last navigated URL (if known).
    pub current_url: Option<String>,
    /// Known user or tenant preferences.
    pub preferences: HashMap<String, String>,
    /// Previously stored memory snippets (free-form hints).
    pub memory_hints: Vec<String>,
    /// Arbitrary metadata for downstream tooling.
    pub metadata: HashMap<String, serde_json::Value>,
}

impl AgentContext {
    pub fn with_session(mut self, session: SessionId) -> Self {
        self.session = Some(session);
        self
    }

    pub fn with_page(mut self, page: PageId) -> Self {
        self.page = Some(page);
        self
    }
}

/// High-level intent metadata inferred from prompts or templates.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AgentIntentMetadata {
    pub intent_id: Option<String>,
    pub primary_goal: Option<String>,
    #[serde(default)]
    pub target_sites: Vec<String>,
    #[serde(default)]
    pub required_outputs: Vec<RequestedOutput>,
    pub preferred_language: Option<String>,
    #[serde(default)]
    pub blocker_remediations: Vec<(String, String)>,
}

/// Structured output requested by the caller.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestedOutput {
    pub schema: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub include_screenshot: bool,
}

impl RequestedOutput {
    pub fn new(schema: impl Into<String>) -> Self {
        Self {
            schema: schema.into(),
            description: None,
            include_screenshot: false,
        }
    }
}

/// Request envelope passed into the agent planner.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentRequest {
    /// Identifier assigned by caller for downstream tracking.
    pub task_id: TaskId,
    /// Natural language goal provided by the user.
    pub goal: String,
    /// Ordered conversation history.
    pub conversation: Vec<ConversationTurn>,
    /// Optional execution context snapshot.
    #[serde(default)]
    pub context: Option<AgentContext>,
    /// Explicit constraints the agent should obey.
    #[serde(default)]
    pub constraints: Vec<String>,
    /// Preferred tool identifiers (hints for planner/tool selection).
    #[serde(default)]
    pub preferred_tools: Vec<String>,
    /// Whether the agent may emit custom tool calls not in the allow-list.
    #[serde(default = "default_allow_custom")]
    pub allow_custom_tools: bool,
    /// Arbitrary metadata for experimentation or caller supplied hints.
    #[serde(default)]
    pub metadata: HashMap<String, serde_json::Value>,
    /// Optional intent metadata populated by the caller.
    #[serde(default)]
    pub intent: AgentIntentMetadata,
}

fn default_allow_custom() -> bool {
    false
}

impl AgentRequest {
    pub fn new(task_id: TaskId, goal: impl Into<String>) -> Self {
        Self {
            task_id,
            goal: goal.into(),
            conversation: Vec::new(),
            context: None,
            constraints: Vec::new(),
            preferred_tools: Vec::new(),
            allow_custom_tools: false,
            metadata: HashMap::new(),
            intent: AgentIntentMetadata::default(),
        }
    }

    pub fn with_context(mut self, context: AgentContext) -> Self {
        self.context = Some(context);
        self
    }

    pub fn push_turn(&mut self, turn: ConversationTurn) {
        self.conversation.push(turn);
    }
}
