//! Gate validator with multi-signal validation

use crate::{conditions::*, errors::GateError, evidence::EvidenceCollector, types::*};
use action_locator::ElementResolver;
use action_primitives::AnchorDescriptor;
use async_trait::async_trait;
use cdp_adapter::{ids::PageId as AdapterPageId, Cdp, CdpAdapter};
use regex::Regex;
use serde_json::Value;
use soulbrowser_core_types::ExecRoute;
use std::sync::Arc;
use std::time::Instant;
use tracing::{debug, info, warn};
use uuid::Uuid;

/// Gate validator trait
#[async_trait]
pub trait GateValidator: Send + Sync {
    /// Validate expectations against current state
    async fn validate(
        &self,
        spec: &ExpectSpec,
        context: &ValidationContext,
        route: &ExecRoute,
    ) -> Result<GateResult, GateError>;

    /// Validate single condition
    async fn validate_condition(
        &self,
        condition: &Condition,
        context: &ValidationContext,
        route: &ExecRoute,
    ) -> Result<bool, GateError>;
}

/// Default gate validator implementation
pub struct DefaultGateValidator {
    pub adapter: Arc<CdpAdapter>,
    pub resolver: Option<Arc<dyn ElementResolver>>,
    evidence_collector: Arc<dyn EvidenceCollector>,
}

impl DefaultGateValidator {
    /// Create a new gate validator
    pub fn new(
        adapter: Arc<CdpAdapter>,
        resolver: Option<Arc<dyn ElementResolver>>,
        evidence_collector: Arc<dyn EvidenceCollector>,
    ) -> Self {
        Self {
            adapter,
            resolver,
            evidence_collector,
        }
    }
}

#[async_trait]
impl GateValidator for DefaultGateValidator {
    async fn validate(
        &self,
        spec: &ExpectSpec,
        context: &ValidationContext,
        route: &ExecRoute,
    ) -> Result<GateResult, GateError> {
        let start = Instant::now();
        info!(
            "Starting gate validation with {} conditions",
            spec.condition_count()
        );

        // Validate spec
        if !spec.has_conditions() {
            warn!("ExpectSpec has no conditions, passing by default");
            return Ok(
                GateResult::pass(vec!["No conditions to validate".to_string()])
                    .with_latency(start.elapsed().as_millis() as u64),
            );
        }

        let mut reasons = Vec::new();
        let mut all_passed = true;

        // Validate "all" conditions (AND logic)
        if !spec.all.is_empty() {
            debug!("Validating {} 'all' conditions", spec.all.len());
            for (i, condition) in spec.all.iter().enumerate() {
                match self.validate_condition(condition, context, route).await {
                    Ok(true) => {
                        debug!("All condition {} passed", i);
                    }
                    Ok(false) => {
                        let reason = format!("All condition {} failed: {:?}", i, condition);
                        warn!("{}", reason);
                        reasons.push(reason);
                        all_passed = false;
                    }
                    Err(e) => {
                        let reason = format!("All condition {} error: {}", i, e);
                        warn!("{}", reason);
                        reasons.push(reason);
                        all_passed = false;
                    }
                }
            }
        }

        // Validate "any" conditions (OR logic)
        if !spec.any.is_empty() {
            debug!("Validating {} 'any' conditions", spec.any.len());
            let mut any_passed = false;
            for (i, condition) in spec.any.iter().enumerate() {
                match self.validate_condition(condition, context, route).await {
                    Ok(true) => {
                        debug!("Any condition {} passed", i);
                        any_passed = true;
                        break;
                    }
                    Ok(false) => {
                        debug!("Any condition {} failed", i);
                    }
                    Err(e) => {
                        warn!("Any condition {} error: {}", i, e);
                    }
                }
            }

            if !any_passed {
                let reason = "None of the 'any' conditions passed".to_string();
                warn!("{}", reason);
                reasons.push(reason);
                all_passed = false;
            }
        }

        // Validate "deny" conditions (NOT logic)
        if !spec.deny.is_empty() {
            debug!("Validating {} 'deny' conditions", spec.deny.len());
            for (i, condition) in spec.deny.iter().enumerate() {
                match self.validate_condition(condition, context, route).await {
                    Ok(true) => {
                        let reason =
                            format!("Deny condition {} passed (should fail): {:?}", i, condition);
                        warn!("{}", reason);
                        reasons.push(reason);
                        all_passed = false;
                    }
                    Ok(false) => {
                        debug!("Deny condition {} failed as expected", i);
                    }
                    Err(e) => {
                        // Error in deny condition counts as "not met" which is good
                        debug!("Deny condition {} error (acceptable): {}", i, e);
                    }
                }
            }
        }

        // Collect evidence
        let evidence = self.evidence_collector.collect_all(context, route).await;

        // Check locator hints if configured
        let locator_hint_result = if !spec.locator_hint.error_indicators.is_empty()
            || !spec.locator_hint.success_indicators.is_empty()
        {
            self.check_locator_hints(&spec.locator_hint, route)
                .await
                .ok()
        } else {
            None
        };

        let latency_ms = start.elapsed().as_millis() as u64;

        // Build result
        let mut result = if all_passed {
            info!("Gate validation passed in {}ms", latency_ms);
            GateResult::pass(if reasons.is_empty() {
                vec!["All conditions met".to_string()]
            } else {
                reasons
            })
        } else {
            info!("Gate validation failed in {}ms", latency_ms);
            GateResult::fail(reasons)
        };

        // Add evidence and metadata
        for e in evidence {
            result = result.with_evidence(e);
        }

        if let Some(hint_result) = locator_hint_result {
            result = result.with_locator_hint(hint_result);
        }

        result = result.with_latency(latency_ms);

        Ok(result)
    }

    async fn validate_condition(
        &self,
        condition: &Condition,
        context: &ValidationContext,
        route: &ExecRoute,
    ) -> Result<bool, GateError> {
        match condition {
            Condition::Dom(dom_cond) => self.validate_dom_condition(dom_cond, context, route).await,
            Condition::Net(net_cond) => self.validate_net_condition(net_cond, context).await,
            Condition::Url(url_cond) => self.validate_url_condition(url_cond, context).await,
            Condition::Title(title_cond) => {
                self.validate_title_condition(title_cond, context).await
            }
            Condition::Runtime(runtime_cond) => {
                self.validate_runtime_condition(runtime_cond, context).await
            }
            Condition::Vis(_) => {
                // TODO: Implement visual condition validation
                debug!("Visual conditions not yet implemented");
                Ok(true)
            }
            Condition::Sem(_) => {
                // TODO: Implement semantic condition validation
                debug!("Semantic conditions not yet implemented");
                Ok(true)
            }
        }
    }
}

impl DefaultGateValidator {
    /// Validate DOM condition
    async fn validate_dom_condition(
        &self,
        condition: &DomCondition,
        context: &ValidationContext,
        route: &ExecRoute,
    ) -> Result<bool, GateError> {
        match condition {
            DomCondition::ElementExists(anchor) => self.element_presence(route, anchor).await,
            DomCondition::ElementNotExists(anchor) => self
                .element_presence(route, anchor)
                .await
                .map(|exists| !exists),
            DomCondition::ElementVisible(anchor) => self.element_visibility(route, anchor).await,
            DomCondition::ElementHidden(anchor) => self
                .element_visibility(route, anchor)
                .await
                .map(|visible| !visible),
            DomCondition::ElementAttribute {
                anchor,
                attribute,
                value,
            } => {
                self.element_attribute(route, anchor, attribute, value)
                    .await
            }
            DomCondition::ElementText {
                anchor,
                text,
                exact,
            } => self.element_text(route, anchor, text, *exact).await,
            DomCondition::MutationCount(count_cond) => {
                Ok(count_cond.matches(context.dom_mutations))
            }
        }
    }

    async fn element_presence(
        &self,
        route: &ExecRoute,
        anchor: &AnchorDescriptor,
    ) -> Result<bool, GateError> {
        let script = dom_probe_script(anchor, "return { found: true };");
        let result = self.run_dom_probe(route, script).await?;
        Ok(result
            .get("found")
            .and_then(Value::as_bool)
            .unwrap_or(false))
    }

    async fn element_visibility(
        &self,
        route: &ExecRoute,
        anchor: &AnchorDescriptor,
    ) -> Result<bool, GateError> {
        let body = r#"
            const style = window.getComputedStyle(el);
            const rect = el.getBoundingClientRect();
            const visible =
                style.visibility !== 'hidden' &&
                style.display !== 'none' &&
                (rect.width > 0 || rect.height > 0 || el.getClientRects().length > 0);
            return { found: true, visible };
        "#;
        let script = dom_probe_script(anchor, body);
        let result = self.run_dom_probe(route, script).await?;
        Ok(result
            .get("visible")
            .and_then(Value::as_bool)
            .unwrap_or(false))
    }

    async fn element_attribute(
        &self,
        route: &ExecRoute,
        anchor: &AnchorDescriptor,
        attribute: &str,
        expected: &Option<String>,
    ) -> Result<bool, GateError> {
        let attr = serde_json::to_string(attribute).unwrap();
        let body = format!(
            "const attrValue = el.getAttribute({}); return {{ found: true, value: attrValue }};",
            attr
        );
        let script = dom_probe_script(anchor, &body);
        let result = self.run_dom_probe(route, script).await?;
        if !result
            .get("found")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            return Ok(false);
        }
        let actual = result.get("value").and_then(Value::as_str);
        Ok(match expected {
            Some(value) => actual.map(|candidate| candidate == value).unwrap_or(false),
            None => actual.is_some(),
        })
    }

    async fn element_text(
        &self,
        route: &ExecRoute,
        anchor: &AnchorDescriptor,
        text: &str,
        exact: bool,
    ) -> Result<bool, GateError> {
        let body = "const content = (el.innerText || el.textContent || ''); return { found: true, text: content };";
        let script = dom_probe_script(anchor, body);
        let result = self.run_dom_probe(route, script).await?;
        if !result
            .get("found")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            return Ok(false);
        }
        let actual = result.get("text").and_then(Value::as_str).unwrap_or("");
        let normalize = |value: &str| value.trim().to_lowercase();
        let expected_norm = normalize(text);
        let actual_norm = normalize(actual);
        Ok(if exact {
            actual_norm == expected_norm
        } else {
            actual_norm.contains(&expected_norm)
        })
    }

    /// Validate network condition
    async fn validate_net_condition(
        &self,
        condition: &NetCondition,
        context: &ValidationContext,
    ) -> Result<bool, GateError> {
        match condition {
            NetCondition::RequestCount(count_cond) => {
                Ok(count_cond.matches(context.network_requests))
            }
            NetCondition::RequestToUrl {
                url_pattern,
                occurred,
            } => {
                // TODO: Check actual network log
                debug!("Checking request to URL pattern: {}", url_pattern);
                Ok(*occurred) // Placeholder
            }
            NetCondition::ResponseStatus {
                url_pattern,
                status_code,
            } => {
                debug!(
                    "Checking response status {} for {}",
                    status_code, url_pattern
                );
                Ok(true) // Placeholder
            }
            NetCondition::NetworkIdle(quiet_ms) => {
                debug!("Checking network idle for {}ms", quiet_ms);
                Ok(true) // Placeholder
            }
        }
    }

    /// Validate URL condition
    async fn validate_url_condition(
        &self,
        condition: &UrlCondition,
        context: &ValidationContext,
    ) -> Result<bool, GateError> {
        let current_url = context
            .current_url
            .as_ref()
            .ok_or_else(|| GateError::MissingSignal("current_url".to_string()))?;

        match condition {
            UrlCondition::Equals(expected) => Ok(current_url == expected),
            UrlCondition::Contains(substring) => Ok(current_url.contains(substring)),
            UrlCondition::Matches(pattern) => {
                let re = Regex::new(pattern)
                    .map_err(|e| GateError::ConditionFailed(format!("Invalid regex: {}", e)))?;
                Ok(re.is_match(current_url))
            }
            UrlCondition::Changed => {
                // TODO: Compare with original URL
                debug!("Checking URL changed");
                Ok(true) // Placeholder
            }
            UrlCondition::Unchanged => {
                debug!("Checking URL unchanged");
                Ok(true) // Placeholder
            }
        }
    }

    /// Validate title condition
    async fn validate_title_condition(
        &self,
        condition: &TitleCondition,
        context: &ValidationContext,
    ) -> Result<bool, GateError> {
        let current_title = context
            .current_title
            .as_ref()
            .ok_or_else(|| GateError::MissingSignal("current_title".to_string()))?;

        match condition {
            TitleCondition::Equals(expected) => Ok(current_title == expected),
            TitleCondition::Contains(substring) => Ok(current_title.contains(substring)),
            TitleCondition::Matches(pattern) => {
                let re = Regex::new(pattern)
                    .map_err(|e| GateError::ConditionFailed(format!("Invalid regex: {}", e)))?;
                Ok(re.is_match(current_title))
            }
            TitleCondition::Changed => {
                debug!("Checking title changed");
                Ok(true) // Placeholder
            }
            TitleCondition::Unchanged => {
                debug!("Checking title unchanged");
                Ok(true) // Placeholder
            }
        }
    }

    /// Validate runtime condition
    async fn validate_runtime_condition(
        &self,
        condition: &RuntimeCondition,
        context: &ValidationContext,
    ) -> Result<bool, GateError> {
        match condition {
            RuntimeCondition::HasErrors => {
                // Check if any console messages contain "error"
                Ok(context
                    .console_messages
                    .iter()
                    .any(|msg| msg.to_lowercase().contains("error")))
            }
            RuntimeCondition::NoErrors => Ok(!context
                .console_messages
                .iter()
                .any(|msg| msg.to_lowercase().contains("error"))),
            RuntimeCondition::MessageMatches(pattern) => {
                let re = Regex::new(pattern)
                    .map_err(|e| GateError::ConditionFailed(format!("Invalid regex: {}", e)))?;
                Ok(context.console_messages.iter().any(|msg| re.is_match(msg)))
            }
            RuntimeCondition::MessageCount(count_cond) => {
                Ok(count_cond.matches(context.console_messages.len() as u32))
            }
            RuntimeCondition::JsEvaluates(expr) => {
                // TODO: Evaluate JavaScript expression via CDP
                debug!("Evaluating JS expression: {}", expr);
                Ok(true) // Placeholder
            }
        }
    }

    /// Check locator hints for suspicious elements
    async fn check_locator_hints(
        &self,
        hint: &LocatorHint,
        _route: &ExecRoute,
    ) -> Result<LocatorHintResult, GateError> {
        // TODO: Implement actual locator hint checking
        debug!(
            "Checking locator hints: {} errors, {} success",
            hint.error_indicators.len(),
            hint.success_indicators.len()
        );

        Ok(LocatorHintResult {
            error_elements: Vec::new(),
            success_elements: Vec::new(),
            appears_successful: true,
        })
    }

    async fn run_dom_probe(&self, route: &ExecRoute, script: String) -> Result<Value, GateError> {
        let page = Self::parse_page_id(route)?;
        self.adapter
            .evaluate_script(page, &script)
            .await
            .map_err(|err| GateError::CdpError(format!("DOM probe failed: {}", err)))
    }

    fn parse_page_id(route: &ExecRoute) -> Result<AdapterPageId, GateError> {
        let id = Uuid::parse_str(&route.page.0)
            .map_err(|err| GateError::Internal(format!("invalid page id: {err}")))?;
        Ok(AdapterPageId(id))
    }
}

fn dom_probe_script(anchor: &AnchorDescriptor, body: &str) -> String {
    let locator = anchor_locator_snippet(anchor);
    format!(
        r#"(() => {{
            {locator}
            if (!el) {{
                return {{ found: false }};
            }}
            const elRef = el;
            {{
                const el = elRef;
                {body}
            }}
        }})()"#
    )
}

fn anchor_locator_snippet(anchor: &AnchorDescriptor) -> String {
    match anchor {
        AnchorDescriptor::Css(selector) => {
            let selector = serde_json::to_string(selector).unwrap_or_else(|_| "''".to_string());
            format!("const el = document.querySelector({selector});")
        }
        AnchorDescriptor::Aria { role, name } => {
            let role = serde_json::to_string(role).unwrap_or_else(|_| "''".to_string());
            let name = serde_json::to_string(name).unwrap_or_else(|_| "''".to_string());
            format!(
                r#"const role = {role};
                    const targetName = {name};
                    const normalize = (value) => (value || '').trim().toLowerCase();
                    const computeName = (node) => {{
                        if (!node) return '';
                        const label = node.getAttribute('aria-label');
                        if (label) return label.trim();
                        const labelledby = node.getAttribute('aria-labelledby');
                        if (labelledby) {{
                            return labelledby.split(/\s+/)
                                .map(id => document.getElementById(id))
                                .map(node => node ? (node.textContent || '') : '')
                                .join(' ')
                                .trim();
                        }}
                        if (node.title) return node.title.trim();
                        return (node.innerText || node.textContent || '').trim();
                    }};
                    const candidates = Array.from(document.querySelectorAll('[role="' + role + '"]'));
                    const el = candidates.find(node => normalize(computeName(node)) === normalize(targetName));"#
            )
        }
        AnchorDescriptor::Text { content, exact } => {
            let pattern = serde_json::to_string(content).unwrap_or_else(|_| "''".to_string());
            let exact_flag = if *exact { "true" } else { "false" };
            format!(
                r#"const searchFor = {pattern};
                    const exact = {exact_flag};
                    const normalize = (value) => (value || '').trim().toLowerCase();
                    const target = normalize(searchFor);
                    const nodes = Array.from(document.querySelectorAll('body *'));
                    const el = nodes.find(node => {{
                        const value = normalize(node.innerText || node.textContent || '');
                        if (!value) return false;
                        return exact ? value === target : value.includes(target);
                    }});"#
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_expect_spec_builder() {
        let spec = ExpectSpec::new()
            .with_timeout(3000)
            .with_all(Condition::Url(UrlCondition::Contains(
                "success".to_string(),
            )))
            .with_any(Condition::Title(TitleCondition::Contains(
                "Complete".to_string(),
            )))
            .with_deny(Condition::Runtime(RuntimeCondition::HasErrors));

        assert_eq!(spec.timeout_ms, 3000);
        assert_eq!(spec.all.len(), 1);
        assert_eq!(spec.any.len(), 1);
        assert_eq!(spec.deny.len(), 1);
        assert!(spec.has_conditions());
        assert_eq!(spec.condition_count(), 3);
    }

    #[test]
    fn test_validation_context() {
        let mut context = ValidationContext::new();
        context.current_url = Some("https://example.com".to_string());
        context.dom_mutations = 5;
        context.add_signal("custom".to_string(), serde_json::json!({"value": 42}));

        assert_eq!(context.current_url, Some("https://example.com".to_string()));
        assert_eq!(context.dom_mutations, 5);
        assert_eq!(context.custom_signals.get("custom").unwrap()["value"], 42);
    }
}
