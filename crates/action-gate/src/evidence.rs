//! Evidence collection system

use crate::types::{Evidence, EvidenceType, ValidationContext};
use cdp_adapter::CdpAdapter;
use serde_json::json;
use soulbrowser_core_types::ExecRoute;
use std::sync::Arc;
use tracing::debug;

/// Evidence collector trait
#[async_trait::async_trait]
pub trait EvidenceCollector: Send + Sync {
    /// Collect all available evidence from validation context
    async fn collect_all(&self, context: &ValidationContext, route: &ExecRoute) -> Vec<Evidence>;

    /// Collect DOM evidence
    async fn collect_dom(&self, route: &ExecRoute) -> Option<Evidence>;

    /// Collect network evidence
    async fn collect_network(&self, context: &ValidationContext) -> Option<Evidence>;

    /// Collect URL evidence
    async fn collect_url(&self, context: &ValidationContext) -> Option<Evidence>;

    /// Collect title evidence
    async fn collect_title(&self, context: &ValidationContext) -> Option<Evidence>;

    /// Collect runtime evidence
    async fn collect_runtime(&self, context: &ValidationContext) -> Option<Evidence>;
}

/// Default evidence collector implementation
pub struct DefaultEvidenceCollector {
    _adapter: Arc<CdpAdapter>,
}

impl DefaultEvidenceCollector {
    /// Create a new evidence collector
    pub fn new(adapter: Arc<CdpAdapter>) -> Self {
        Self { _adapter: adapter }
    }
}

#[async_trait::async_trait]
impl EvidenceCollector for DefaultEvidenceCollector {
    async fn collect_all(&self, context: &ValidationContext, route: &ExecRoute) -> Vec<Evidence> {
        let mut evidence = Vec::new();

        // Collect from all sources
        if let Some(e) = self.collect_dom(route).await {
            evidence.push(e);
        }
        if let Some(e) = self.collect_network(context).await {
            evidence.push(e);
        }
        if let Some(e) = self.collect_url(context).await {
            evidence.push(e);
        }
        if let Some(e) = self.collect_title(context).await {
            evidence.push(e);
        }
        if let Some(e) = self.collect_runtime(context).await {
            evidence.push(e);
        }

        evidence
    }

    async fn collect_dom(&self, _route: &ExecRoute) -> Option<Evidence> {
        debug!("Collecting DOM evidence");

        // TODO: Implement actual DOM snapshot collection via CDP
        // For now, return placeholder

        Some(Evidence::new(
            EvidenceType::Dom,
            "DOM snapshot".to_string(),
            json!({
                "node_count": 0,
                "mutations": 0,
            }),
        ))
    }

    async fn collect_network(&self, context: &ValidationContext) -> Option<Evidence> {
        debug!("Collecting network evidence");

        Some(Evidence::new(
            EvidenceType::Network,
            "Network activity summary".to_string(),
            json!({
                "request_count": context.network_requests,
                "requests": [],
            }),
        ))
    }

    async fn collect_url(&self, context: &ValidationContext) -> Option<Evidence> {
        debug!("Collecting URL evidence");

        context.current_url.as_ref().map(|url| {
            Evidence::new(
                EvidenceType::Url,
                "Current URL".to_string(),
                json!({
                    "url": url,
                }),
            )
        })
    }

    async fn collect_title(&self, context: &ValidationContext) -> Option<Evidence> {
        debug!("Collecting title evidence");

        context.current_title.as_ref().map(|title| {
            Evidence::new(
                EvidenceType::Title,
                "Current title".to_string(),
                json!({
                    "title": title,
                }),
            )
        })
    }

    async fn collect_runtime(&self, context: &ValidationContext) -> Option<Evidence> {
        debug!("Collecting runtime evidence");

        if context.console_messages.is_empty() {
            return None;
        }

        Some(Evidence::new(
            EvidenceType::Runtime,
            "Console messages".to_string(),
            json!({
                "message_count": context.console_messages.len(),
                "messages": context.console_messages,
            }),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_evidence_creation() {
        let evidence = Evidence::new(
            EvidenceType::Dom,
            "Test evidence".to_string(),
            json!({"key": "value"}),
        );

        assert_eq!(evidence.evidence_type, EvidenceType::Dom);
        assert_eq!(evidence.description, "Test evidence");
        assert_eq!(evidence.value["key"], "value");
    }

    #[test]
    fn test_evidence_type_names() {
        assert_eq!(EvidenceType::Dom.name(), "dom");
        assert_eq!(EvidenceType::Network.name(), "network");
        assert_eq!(EvidenceType::Url.name(), "url");
        assert_eq!(EvidenceType::Title.name(), "title");
        assert_eq!(EvidenceType::Runtime.name(), "runtime");
    }
}
