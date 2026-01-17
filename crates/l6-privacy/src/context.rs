use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum RedactScope {
    Observation,
    Event,
    StateCenter,
    Export,
    Screenshot,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RedactCtx {
    pub trace_id: Option<String>,
    pub span_id: Option<String>,
    pub action_id: Option<String>,
    pub scope: RedactScope,
    pub origin: Option<String>,
    pub tags: BTreeSet<String>,
    pub export: bool,
}

impl RedactCtx {
    pub fn with_tag<T: Into<String>>(mut self, tag: T) -> Self {
        self.tags.insert(tag.into());
        self
    }

    pub fn tag_matches(&self, needle: &str) -> bool {
        self.tags.iter().any(|tag| tag == needle)
    }
}

impl Default for RedactScope {
    fn default() -> Self {
        RedactScope::Observation
    }
}
