use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

#[derive(Debug, Serialize)]
pub struct StatusResponse {
    pub value: StatusValue,
}

#[derive(Debug, Serialize)]
pub struct StatusValue {
    pub ready: bool,
    pub message: String,
}

#[derive(Debug, Serialize)]
pub struct SessionCreated {
    pub session_id: String,
    pub capabilities: Value,
}

#[derive(Debug, Deserialize)]
pub struct NewSessionRequest {
    pub capabilities: Option<Value>,
}

#[derive(Debug, Deserialize)]
pub struct NavigateToUrlRequest {
    pub url: String,
}

#[derive(Debug, Deserialize)]
pub struct FindElementRequest {
    pub using: String,
    pub value: String,
}

impl SessionCreated {
    pub fn new() -> Self {
        Self {
            session_id: Uuid::new_v4().to_string(),
            capabilities: Value::default(),
        }
    }
}
