//! Condition types for post-action validation

use action_primitives::AnchorDescriptor;
use serde::{Deserialize, Serialize};

/// Condition enumeration for validation
///
/// Five main signal types: DOM, Network, URL, Title, Runtime
/// Plus optional Visual and Semantic conditions
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Condition {
    /// DOM-based condition
    Dom(DomCondition),

    /// Network-based condition
    Net(NetCondition),

    /// URL-based condition
    Url(UrlCondition),

    /// Title-based condition
    Title(TitleCondition),

    /// Runtime/console-based condition
    Runtime(RuntimeCondition),

    /// Visual-based condition (optional, requires visual perceiver)
    Vis(VisCondition),

    /// Semantic-based condition (optional, requires semantic perceiver)
    Sem(SemCondition),
}

/// DOM condition types
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DomCondition {
    /// Element exists matching anchor
    ElementExists(AnchorDescriptor),

    /// Element does not exist matching anchor
    ElementNotExists(AnchorDescriptor),

    /// Element is visible
    ElementVisible(AnchorDescriptor),

    /// Element is hidden
    ElementHidden(AnchorDescriptor),

    /// Element has specific attribute value
    ElementAttribute {
        anchor: AnchorDescriptor,
        attribute: String,
        value: Option<String>, // None = just check existence
    },

    /// Element has specific text content
    ElementText {
        anchor: AnchorDescriptor,
        text: String,
        exact: bool,
    },

    /// DOM mutation count matches condition
    MutationCount(CountCondition),
}

/// Network condition types
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum NetCondition {
    /// Request count matches condition
    RequestCount(CountCondition),

    /// Request to specific URL occurred
    RequestToUrl {
        url_pattern: String,
        occurred: bool, // true = must occur, false = must not occur
    },

    /// Response status code matches
    ResponseStatus {
        url_pattern: String,
        status_code: u16,
    },

    /// Network idle (no requests for N ms)
    NetworkIdle(u64),
}

/// URL condition types
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum UrlCondition {
    /// URL matches exact string
    Equals(String),

    /// URL contains substring
    Contains(String),

    /// URL matches regex pattern
    Matches(String),

    /// URL has changed from original
    Changed,

    /// URL has not changed from original
    Unchanged,
}

/// Title condition types
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TitleCondition {
    /// Title matches exact string
    Equals(String),

    /// Title contains substring
    Contains(String),

    /// Title matches regex pattern
    Matches(String),

    /// Title has changed from original
    Changed,

    /// Title has not changed from original
    Unchanged,
}

/// Runtime/console condition types
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RuntimeCondition {
    /// Console has error messages
    HasErrors,

    /// Console has no error messages
    NoErrors,

    /// Console message matches pattern
    MessageMatches(String),

    /// Console message count matches condition
    MessageCount(CountCondition),

    /// JavaScript expression evaluates to true
    JsEvaluates(String),
}

/// Visual condition types (requires visual perceiver)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum VisCondition {
    /// Visual diff from baseline is below threshold
    DiffBelow(f64),

    /// Specific color is present
    ColorPresent { r: u8, g: u8, b: u8 },

    /// Screenshot matches expected (hash comparison)
    ScreenshotMatches(String),
}

/// Semantic condition types (requires semantic perceiver)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SemCondition {
    /// Page language matches
    LanguageIs(String),

    /// Content type matches
    ContentType(String),

    /// Page intent matches
    Intent(String),

    /// Keywords present
    KeywordsPresent(Vec<String>),
}

/// Count condition for numeric comparisons
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CountCondition {
    /// Equal to value
    Equals(u32),

    /// Greater than value
    GreaterThan(u32),

    /// Less than value
    LessThan(u32),

    /// Between min and max (inclusive)
    Between(u32, u32),
}

impl CountCondition {
    /// Check if count matches condition
    pub fn matches(&self, count: u32) -> bool {
        match self {
            CountCondition::Equals(expected) => count == *expected,
            CountCondition::GreaterThan(threshold) => count > *threshold,
            CountCondition::LessThan(threshold) => count < *threshold,
            CountCondition::Between(min, max) => count >= *min && count <= *max,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_count_condition() {
        assert!(CountCondition::Equals(5).matches(5));
        assert!(!CountCondition::Equals(5).matches(4));

        assert!(CountCondition::GreaterThan(3).matches(4));
        assert!(!CountCondition::GreaterThan(3).matches(3));

        assert!(CountCondition::LessThan(10).matches(9));
        assert!(!CountCondition::LessThan(10).matches(10));

        assert!(CountCondition::Between(5, 10).matches(7));
        assert!(CountCondition::Between(5, 10).matches(5));
        assert!(CountCondition::Between(5, 10).matches(10));
        assert!(!CountCondition::Between(5, 10).matches(4));
        assert!(!CountCondition::Between(5, 10).matches(11));
    }
}
