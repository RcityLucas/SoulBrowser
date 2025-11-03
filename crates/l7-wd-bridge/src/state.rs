use parking_lot::RwLock;
use serde_json::Value;
use std::collections::HashMap;
use std::time::Instant;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct ElementModel {
    pub session_id: String,
    pub using: String,
    pub value: String,
    pub last_seen_text: Option<String>,
    pub attributes: HashMap<String, String>,
}

impl ElementModel {
    pub fn new(session_id: String, using: String, value: String) -> Self {
        Self {
            session_id,
            using,
            value,
            last_seen_text: None,
            attributes: HashMap::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct SessionModel {
    pub session_id: String,
    pub tenant_id: String,
    pub capabilities: Value,
    pub created_at: Instant,
    pub next_element_id: u64,
    pub current_url: Option<String>,
}

impl SessionModel {
    pub fn new(tenant_id: String, capabilities: Value) -> Self {
        Self {
            session_id: Uuid::new_v4().to_string(),
            tenant_id,
            capabilities,
            created_at: Instant::now(),
            next_element_id: 1,
            current_url: None,
        }
    }
}

#[derive(Debug, Default)]
pub struct SessionStore {
    sessions: RwLock<HashMap<String, SessionModel>>,
    elements: RwLock<HashMap<String, ElementModel>>, // element id -> metadata
}

impl SessionStore {
    pub fn create(&self, tenant_id: String, capabilities: Value) -> SessionModel {
        let model = SessionModel::new(tenant_id, capabilities);
        self.sessions
            .write()
            .insert(model.session_id.clone(), model.clone());
        model
    }

    pub fn get(&self, id: &str) -> Option<SessionModel> {
        self.sessions.read().get(id).cloned()
    }

    pub fn remove(&self, id: &str) {
        self.sessions.write().remove(id);
        self.elements
            .write()
            .retain(|_, element| element.session_id != id);
    }

    pub fn allocate_element(&self, session_id: &str, using: &str, value: &str) -> Option<String> {
        let mut sessions = self.sessions.write();
        let session = sessions.get_mut(session_id)?;
        let element_id = format!("element-{}", session.next_element_id);
        session.next_element_id += 1;
        drop(sessions);
        self.elements.write().insert(
            element_id.clone(),
            ElementModel::new(session_id.to_string(), using.to_string(), value.to_string()),
        );
        Some(element_id)
    }

    pub fn element_session(&self, element_id: &str) -> Option<String> {
        self.elements
            .read()
            .get(element_id)
            .map(|entry| entry.session_id.clone())
    }

    pub fn element_entry(&self, element_id: &str) -> Option<ElementModel> {
        self.elements.read().get(element_id).cloned()
    }

    pub fn update_element_text(&self, element_id: &str, text: &str) {
        if let Some(entry) = self.elements.write().get_mut(element_id) {
            entry.last_seen_text = Some(text.to_string());
        }
    }

    pub fn update_element_attribute(&self, element_id: &str, name: &str, value: &str) {
        if let Some(entry) = self.elements.write().get_mut(element_id) {
            entry.attributes.insert(name.to_string(), value.to_string());
        }
    }

    pub fn set_current_url(&self, session_id: &str, url: String) {
        if let Some(session) = self.sessions.write().get_mut(session_id) {
            session.current_url = Some(url);
        }
    }

    pub fn current_url(&self, session_id: &str) -> Option<String> {
        self.sessions
            .read()
            .get(session_id)
            .and_then(|session| session.current_url.clone())
    }
}
