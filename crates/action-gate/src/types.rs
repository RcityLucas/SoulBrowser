//! Core types for post-conditions gate

use crate::conditions::Condition;
use action_locator::LocatorStrategy;
use action_primitives::AnchorDescriptor;
use serde::{Deserialize, Serialize};

/// ExpectSpec - Rule model for post-condition validation
///
/// Defines expectations that must be met after an action completes.
/// Uses three rule categories: all (AND), any (OR), deny (NOT).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExpectSpec {
    /// Timeout for validation in milliseconds
    pub timeout_ms: u64,

    /// All conditions must pass (AND logic)
    pub all: Vec<Condition>,

    /// At least one condition must pass (OR logic)
    pub any: Vec<Condition>,

    /// None of these conditions should pass (NOT logic)
    pub deny: Vec<Condition>,

    /// Hint for suspicious element detection
    pub locator_hint: LocatorHint,
}

impl ExpectSpec {
    /// Create a new ExpectSpec with default timeout
    pub fn new() -> Self {
        Self {
            timeout_ms: 5000, // 5 seconds default
            all: Vec::new(),
            any: Vec::new(),
            deny: Vec::new(),
            locator_hint: LocatorHint::default(),
        }
    }

    /// Set timeout
    pub fn with_timeout(mut self, timeout_ms: u64) -> Self {
        self.timeout_ms = timeout_ms;
        self
    }

    /// Add "all" condition (AND)
    pub fn with_all(mut self, condition: Condition) -> Self {
        self.all.push(condition);
        self
    }

    /// Add "any" condition (OR)
    pub fn with_any(mut self, condition: Condition) -> Self {
        self.any.push(condition);
        self
    }

    /// Add "deny" condition (NOT)
    pub fn with_deny(mut self, condition: Condition) -> Self {
        self.deny.push(condition);
        self
    }

    /// Set locator hint
    pub fn with_locator_hint(mut self, hint: LocatorHint) -> Self {
        self.locator_hint = hint;
        self
    }

    /// Check if spec has any conditions
    pub fn has_conditions(&self) -> bool {
        !self.all.is_empty() || !self.any.is_empty() || !self.deny.is_empty()
    }

    /// Get total condition count
    pub fn condition_count(&self) -> usize {
        self.all.len() + self.any.len() + self.deny.len()
    }
}

impl Default for ExpectSpec {
    fn default() -> Self {
        Self::new()
    }
}

/// Locator hint for detecting suspicious elements
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LocatorHint {
    /// Look for error indicators
    pub error_indicators: Vec<String>,

    /// Look for success indicators
    pub success_indicators: Vec<String>,

    /// Strategies to try for detection
    pub strategies: Vec<LocatorStrategy>,
}

impl LocatorHint {
    /// Create a new locator hint
    pub fn new() -> Self {
        Self::default()
    }

    /// Add error indicator
    pub fn with_error(mut self, indicator: String) -> Self {
        self.error_indicators.push(indicator);
        self
    }

    /// Add success indicator
    pub fn with_success(mut self, indicator: String) -> Self {
        self.success_indicators.push(indicator);
        self
    }

    /// Add strategy
    pub fn with_strategy(mut self, strategy: LocatorStrategy) -> Self {
        self.strategies.push(strategy);
        self
    }
}

/// Gate validation result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GateResult {
    /// Whether validation passed
    pub passed: bool,

    /// Reasons for pass/fail
    pub reasons: Vec<String>,

    /// Evidence collected during validation
    pub evidence: Vec<Evidence>,

    /// Locator hint result (if suspicious elements detected)
    pub locator_hint_result: Option<LocatorHintResult>,

    /// Validation latency in milliseconds
    pub latency_ms: u64,
}

impl GateResult {
    /// Create a passing result
    pub fn pass(reasons: Vec<String>) -> Self {
        Self {
            passed: true,
            reasons,
            evidence: Vec::new(),
            locator_hint_result: None,
            latency_ms: 0,
        }
    }

    /// Create a failing result
    pub fn fail(reasons: Vec<String>) -> Self {
        Self {
            passed: false,
            reasons,
            evidence: Vec::new(),
            locator_hint_result: None,
            latency_ms: 0,
        }
    }

    /// Add evidence
    pub fn with_evidence(mut self, evidence: Evidence) -> Self {
        self.evidence.push(evidence);
        self
    }

    /// Set locator hint result
    pub fn with_locator_hint(mut self, hint: LocatorHintResult) -> Self {
        self.locator_hint_result = Some(hint);
        self
    }

    /// Set latency
    pub fn with_latency(mut self, latency_ms: u64) -> Self {
        self.latency_ms = latency_ms;
        self
    }
}

/// Locator hint result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocatorHintResult {
    /// Error elements found
    pub error_elements: Vec<SuspiciousElement>,

    /// Success elements found
    pub success_elements: Vec<SuspiciousElement>,

    /// Overall verdict (true = success indicators dominate)
    pub appears_successful: bool,
}

/// Suspicious element detected via locator hint
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SuspiciousElement {
    /// Element descriptor
    pub anchor: AnchorDescriptor,

    /// Matching indicator text
    pub indicator: String,

    /// Element text content
    pub text_content: Option<String>,

    /// Confidence score
    pub confidence: f64,
}

/// Evidence piece from validation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Evidence {
    /// Evidence type
    pub evidence_type: EvidenceType,

    /// Evidence description
    pub description: String,

    /// Evidence value (JSON-serializable)
    pub value: serde_json::Value,

    /// Timestamp when evidence was collected
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

impl Evidence {
    /// Create new evidence
    pub fn new(evidence_type: EvidenceType, description: String, value: serde_json::Value) -> Self {
        Self {
            evidence_type,
            description,
            value,
            timestamp: chrono::Utc::now(),
        }
    }
}

/// Evidence type enumeration
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EvidenceType {
    /// DOM-related evidence
    Dom,

    /// Network-related evidence
    Network,

    /// URL-related evidence
    Url,

    /// Title-related evidence
    Title,

    /// Runtime/console evidence
    Runtime,

    /// Visual evidence
    Visual,

    /// Semantic evidence
    Semantic,
}

impl EvidenceType {
    /// Get evidence type name
    pub fn name(&self) -> &'static str {
        match self {
            EvidenceType::Dom => "dom",
            EvidenceType::Network => "network",
            EvidenceType::Url => "url",
            EvidenceType::Title => "title",
            EvidenceType::Runtime => "runtime",
            EvidenceType::Visual => "visual",
            EvidenceType::Semantic => "semantic",
        }
    }
}

/// Validation context for gate checking
#[derive(Debug, Clone)]
pub struct ValidationContext {
    /// Current URL
    pub current_url: Option<String>,

    /// Current title
    pub current_title: Option<String>,

    /// DOM mutation count
    pub dom_mutations: u32,

    /// Network requests count
    pub network_requests: u32,

    /// Console messages
    pub console_messages: Vec<String>,

    /// Custom signals (extensible)
    pub custom_signals: std::collections::HashMap<String, serde_json::Value>,
}

impl ValidationContext {
    /// Create a new validation context
    pub fn new() -> Self {
        Self {
            current_url: None,
            current_title: None,
            dom_mutations: 0,
            network_requests: 0,
            console_messages: Vec::new(),
            custom_signals: std::collections::HashMap::new(),
        }
    }

    /// Add custom signal
    pub fn add_signal(&mut self, key: String, value: serde_json::Value) {
        self.custom_signals.insert(key, value);
    }
}

impl Default for ValidationContext {
    fn default() -> Self {
        Self::new()
    }
}
