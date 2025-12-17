///! Data models for multi-modal perception
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// Re-export perceiver models
pub use perceiver_semantic::{ContentType, PageIntent};
pub use perceiver_structural::model::SnapshotId;
pub use perceiver_visual::{Screenshot, VisualDiffResult, VisualMetricsResult};

/// Multi-modal perception result combining all perceiver outputs
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MultiModalPerception {
    /// Structural analysis results
    pub structural: StructuralAnalysis,

    /// Visual analysis results (optional)
    pub visual: Option<VisualAnalysis>,

    /// Semantic analysis results (optional)
    pub semantic: Option<SemanticAnalysis>,

    /// Cross-modal insights
    pub insights: Vec<CrossModalInsight>,

    /// Overall confidence score (0.0-1.0)
    pub confidence: f64,
}

/// Structural analysis summary
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StructuralAnalysis {
    /// Snapshot ID
    pub snapshot_id: String,

    /// Number of DOM nodes
    pub dom_node_count: usize,

    /// Number of interactive elements
    pub interactive_element_count: usize,

    /// Page has forms
    pub has_forms: bool,

    /// Page has navigation
    pub has_navigation: bool,
}

impl StructuralAnalysis {
    /// Create a placeholder entry representing disabled structural analysis
    pub fn disabled() -> Self {
        Self {
            snapshot_id: "structural-disabled".to_string(),
            dom_node_count: 0,
            interactive_element_count: 0,
            has_forms: false,
            has_navigation: false,
        }
    }
}

impl Default for StructuralAnalysis {
    fn default() -> Self {
        Self::disabled()
    }
}

/// Visual analysis summary
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VisualAnalysis {
    /// Screenshot ID
    pub screenshot_id: String,

    /// Dominant colors (top 5)
    pub dominant_colors: Vec<(u8, u8, u8)>,

    /// Average contrast ratio
    pub avg_contrast: f64,

    /// Viewport utilization (0.0-1.0)
    pub viewport_utilization: f64,

    /// Visual complexity score (0.0-1.0)
    pub complexity: f64,
}

/// Semantic analysis summary
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SemanticAnalysis {
    /// Content type classification
    pub content_type: ContentType,

    /// Page intent
    pub intent: PageIntent,

    /// Primary language
    pub language: String,

    /// Language confidence (0.0-1.0)
    pub language_confidence: f64,

    /// Content summary (short)
    pub summary: String,

    /// Top keywords
    pub keywords: Vec<String>,

    /// Readability score (0.0-100.0)
    pub readability: Option<f64>,
}

/// Cross-modal insight from combining multiple perceivers
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrossModalInsight {
    /// Insight type
    pub insight_type: InsightType,

    /// Insight description
    pub description: String,

    /// Confidence score (0.0-1.0)
    pub confidence: f64,

    /// Contributing perceivers
    pub sources: Vec<PerceiverType>,
}

/// Types of cross-modal insights
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum InsightType {
    /// Content and structure alignment
    ContentStructureAlignment,

    /// Visual and semantic consistency
    VisualSemanticConsistency,

    /// Accessibility issue detected
    AccessibilityIssue,

    /// User experience observation
    UserExperience,

    /// Performance consideration
    Performance,

    /// Security observation
    Security,
}

/// Types of perceivers
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PerceiverType {
    Structural,
    Visual,
    Semantic,
}

/// Options for multi-modal perception
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerceptionOptions {
    /// Enable structural analysis
    pub enable_structural: bool,

    /// Enable visual analysis
    pub enable_visual: bool,

    /// Enable semantic analysis
    pub enable_semantic: bool,

    /// Enable cross-modal insights
    pub enable_insights: bool,

    /// Capture screenshot for visual analysis
    pub capture_screenshot: bool,

    /// Extract text for semantic analysis
    pub extract_text: bool,

    /// Timeout for analysis (seconds)
    pub timeout_secs: u64,
}

impl Default for PerceptionOptions {
    fn default() -> Self {
        Self {
            enable_structural: true,
            enable_visual: true,
            enable_semantic: true,
            enable_insights: true,
            capture_screenshot: true,
            extract_text: true,
            timeout_secs: 30,
        }
    }
}

/// Element information from multi-modal analysis
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MultiModalElement {
    /// Element identifier
    pub id: String,

    /// Structural properties
    pub structural: Option<StructuralProperties>,

    /// Visual properties
    pub visual: Option<VisualProperties>,

    /// Semantic properties
    pub semantic: Option<SemanticProperties>,

    /// Confidence score (0.0-1.0)
    pub confidence: f64,
}

/// Structural element properties
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StructuralProperties {
    /// Tag name
    pub tag_name: String,

    /// Attributes
    pub attributes: HashMap<String, String>,

    /// Is interactive
    pub is_interactive: bool,

    /// Is visible
    pub is_visible: bool,
}

/// Visual element properties
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VisualProperties {
    /// Bounding box (x, y, width, height)
    pub bounds: (f64, f64, f64, f64),

    /// Background color
    pub bg_color: Option<(u8, u8, u8)>,

    /// Is occluded
    pub is_occluded: bool,

    /// Visual prominence (0.0-1.0)
    pub prominence: f64,
}

/// Semantic element properties
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SemanticProperties {
    /// Text content
    pub text: String,

    /// Semantic role
    pub role: String,

    /// Associated intent
    pub intent: Option<String>,
}
