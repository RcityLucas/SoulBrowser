use crate::{
    errors::SoulBrowserError,
    policy::{merge_attrs, BrowserPolicy},
};
use serde_json::json;
use soulbase_auth::{
    model::{Action, AuthnInput, Decision, QuotaKey, QuotaOutcome},
    AuthFacade,
};
use soulbase_types::{
    id::Id,
    subject::{Subject, SubjectKind},
    tenant::TenantId,
};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Clone)]
pub struct AuthSession {
    subject: Subject,
    authn_input: AuthnInput,
}

impl AuthSession {
    pub fn new(subject: Subject, authn_input: AuthnInput) -> Self {
        Self {
            subject,
            authn_input,
        }
    }

    pub fn subject(&self) -> &Subject {
        &self.subject
    }

    pub fn authn_input(&self) -> AuthnInput {
        self.authn_input.clone()
    }
}

pub struct BrowserAuthManager {
    tenant: TenantId,
    policy: BrowserPolicy,
    facade: Arc<AuthFacade>,
}

impl BrowserAuthManager {
    pub async fn new(tenant_id: String) -> Result<Self, SoulBrowserError> {
        Self::with_policy_paths(tenant_id, &[]).await
    }

    pub async fn with_policy_paths(
        tenant_id: String,
        policy_paths: &[PathBuf],
    ) -> Result<Self, SoulBrowserError> {
        let policy = BrowserPolicy::load_with_paths(policy_paths).await?;
        let facade = AuthFacade::minimal();

        Ok(Self {
            tenant: TenantId(tenant_id),
            policy,
            facade: Arc::new(facade),
        })
    }

    pub async fn authenticate_token(
        &self,
        subject_id: String,
    ) -> Result<AuthSession, SoulBrowserError> {
        let subject = Subject {
            kind: SubjectKind::User,
            subject_id: Id(subject_id.clone()),
            tenant: self.tenant.clone(),
            claims: Default::default(),
        };

        let input = AuthnInput::BearerJwt(subject_id);
        Ok(AuthSession::new(subject, input))
    }

    #[allow(dead_code)]
    pub async fn authorize_request(
        &self,
        session: &AuthSession,
        method: &str,
        path: &str,
        session_id: &str,
        body: Option<&serde_json::Value>,
    ) -> Result<Decision, SoulBrowserError> {
        let policy = self.policy.route_policy();
        let Some(binding) = policy.match_http(method, path) else {
            return Err(SoulBrowserError::forbidden(&format!(
                "Route not allowed: {} {}",
                method, path
            )));
        };

        let mut attrs = binding
            .bind
            .attrs_template
            .clone()
            .unwrap_or_else(|| json!({}));

        if binding.bind.attrs_from_body {
            if let Some(payload) = body {
                merge_attrs(&mut attrs, payload.clone());
            }
        }

        merge_attrs(
            &mut attrs,
            json!({
                "browser": {
                    "tenant": session.subject().tenant.0.clone(),
                    "session": session_id,
                    "channel": "automation",
                },
                "subject": {
                    "id": session.subject().subject_id.0.clone(),
                },
                "timestamp": chrono::Utc::now().timestamp_millis(),
            }),
        );

        let decision = self
            .facade
            .authorize(
                session.authn_input(),
                soulbase_auth::model::ResourceUrn(binding.bind.resource.clone()),
                route_action_from_str(&binding.bind.action),
                attrs,
                None,
                None,
            )
            .await
            .map_err(|err| SoulBrowserError::auth_error(&err.to_string()))?;

        Ok(decision)
    }

    pub fn route_policy(&self) -> soulbase_interceptors::policy::dsl::RoutePolicy {
        self.policy.route_policy()
    }

    pub fn auth_facade(&self) -> Arc<AuthFacade> {
        self.facade.clone()
    }

    #[allow(dead_code)]
    pub async fn check_quota(
        &self,
        session: &AuthSession,
        resource: &str,
        action: BrowserAction,
    ) -> Result<QuotaOutcome, SoulBrowserError> {
        let quota_key = QuotaKey {
            tenant: session.subject().tenant.clone(),
            subject_id: session.subject().subject_id.clone(),
            resource: soulbase_auth::model::ResourceUrn(resource.to_string()),
            action: Action::from(action),
        };

        self.facade
            .quota
            .check_and_consume(&quota_key, 1)
            .await
            .map_err(|err| SoulBrowserError::auth_error(&err.to_string()))
    }
}

fn route_action_from_str(action: &str) -> Action {
    match action.to_ascii_lowercase().as_str() {
        "read" => Action::Read,
        "write" => Action::Write,
        "list" => Action::List,
        "admin" => Action::Admin,
        _ => Action::Invoke,
    }
}

/// Browser action types used for quota enforcement.
#[derive(Clone, Copy)]
pub enum BrowserAction {
    Read,
    Write,
    Execute,
    List,
    Admin,
}

impl From<BrowserAction> for Action {
    fn from(value: BrowserAction) -> Self {
        match value {
            BrowserAction::Read => Action::Read,
            BrowserAction::Write => Action::Write,
            BrowserAction::Execute => Action::Invoke,
            BrowserAction::List => Action::List,
            BrowserAction::Admin => Action::Admin,
        }
    }
}

pub struct SessionManager {
    sessions: Arc<RwLock<HashMap<String, SessionData>>>,
}

#[derive(Clone)]
struct SessionData {
    #[allow(dead_code)]
    auth_session: AuthSession,
}

impl SessionManager {
    pub fn new() -> Self {
        Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn create_session(&self, auth_session: AuthSession) -> String {
        let session_id = uuid::Uuid::new_v4().to_string();
        let mut sessions = self.sessions.write().await;
        sessions.insert(session_id.clone(), SessionData { auth_session });
        session_id
    }
}
