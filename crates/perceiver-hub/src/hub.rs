///! Multi-modal perception hub implementation
use crate::{errors::*, models::*};
use async_trait::async_trait;
use perceiver_semantic::{SemanticOptions, SemanticPerceiver};
use perceiver_structural::StructuralPerceiver;
use perceiver_visual::{ScreenshotOptions, VisualPerceiver};
use soulbrowser_core_types::ExecRoute;
use std::sync::Arc;
use std::time::Duration;

/// Multi-modal perception hub trait
#[async_trait]
pub trait PerceptionHub: Send + Sync {
    /// Perform multi-modal analysis of a page
    async fn perceive(
        &self,
        route: &ExecRoute,
        options: PerceptionOptions,
    ) -> Result<MultiModalPerception>;

    /// Get structural perceiver
    fn structural(&self) -> Arc<dyn StructuralPerceiver>;

    /// Get visual perceiver
    fn visual(&self) -> Option<Arc<dyn VisualPerceiver>>;

    /// Get semantic perceiver
    fn semantic(&self) -> Option<Arc<dyn SemanticPerceiver>>;
}

/// Multi-modal perception hub implementation
pub struct PerceptionHubImpl {
    structural_perceiver: Arc<dyn StructuralPerceiver>,
    visual_perceiver: Option<Arc<dyn VisualPerceiver>>,
    semantic_perceiver: Option<Arc<dyn SemanticPerceiver>>,
}

impl PerceptionHubImpl {
    /// Create new perception hub with all perceivers
    pub fn new(
        structural: Arc<dyn StructuralPerceiver>,
        visual: Arc<dyn VisualPerceiver>,
        semantic: Arc<dyn SemanticPerceiver>,
    ) -> Self {
        Self {
            structural_perceiver: structural,
            visual_perceiver: Some(visual),
            semantic_perceiver: Some(semantic),
        }
    }

    /// Create hub with only structural perceiver
    pub fn structural_only(structural: Arc<dyn StructuralPerceiver>) -> Self {
        Self {
            structural_perceiver: structural,
            visual_perceiver: None,
            semantic_perceiver: None,
        }
    }

    /// Add visual perceiver to hub
    pub fn with_visual(mut self, visual: Arc<dyn VisualPerceiver>) -> Self {
        self.visual_perceiver = Some(visual);
        self
    }

    /// Add semantic perceiver to hub
    pub fn with_semantic(mut self, semantic: Arc<dyn SemanticPerceiver>) -> Self {
        self.semantic_perceiver = Some(semantic);
        self
    }

    /// Analyze structure
    async fn analyze_structural(&self, route: &ExecRoute) -> Result<StructuralAnalysis> {
        // Get DOM snapshot
        let snapshot = self
            .structural_perceiver
            .snapshot_dom_ax(route.clone())
            .await?;

        // Extract basic metrics from snapshot
        // Note: This is simplified - in production, we'd parse the DOM more thoroughly
        let dom_node_count = if let Some(nodes) = snapshot.dom_raw.get("nodes") {
            nodes.as_array().map(|a| a.len()).unwrap_or(0)
        } else {
            0
        };

        Ok(StructuralAnalysis {
            snapshot_id: snapshot.id.0.clone(),
            dom_node_count,
            interactive_element_count: 0, // TODO: Count from DOM
            has_forms: false,             // TODO: Detect from DOM
            has_navigation: false,        // TODO: Detect from DOM
        })
    }

    /// Analyze visuals
    async fn analyze_visual(
        &self,
        route: &ExecRoute,
        capture: bool,
    ) -> Result<Option<VisualAnalysis>> {
        let visual = match &self.visual_perceiver {
            Some(v) => v,
            None => return Ok(None),
        };

        if !capture {
            return Ok(None);
        }

        // Capture screenshot
        let screenshot = visual
            .capture_screenshot(route, ScreenshotOptions::default())
            .await?;

        // Analyze visual metrics
        let metrics = visual.analyze_metrics(&screenshot).await?;

        Ok(Some(VisualAnalysis {
            screenshot_id: screenshot.id.clone(),
            dominant_colors: metrics.color_palette.clone(),
            avg_contrast: metrics.avg_contrast_ratio,
            viewport_utilization: metrics.viewport_utilization,
            complexity: Self::calculate_visual_complexity(&metrics),
        }))
    }

    /// Analyze semantics
    async fn analyze_semantic(
        &self,
        route: &ExecRoute,
        extract: bool,
    ) -> Result<Option<SemanticAnalysis>> {
        let semantic = match &self.semantic_perceiver {
            Some(s) => s,
            None => return Ok(None),
        };

        if !extract {
            return Ok(None);
        }

        // Perform semantic analysis
        let analysis = semantic.analyze(route, SemanticOptions::default()).await?;

        Ok(Some(SemanticAnalysis {
            content_type: analysis.content_type,
            intent: analysis.intent,
            language: analysis.language.code.clone(),
            language_confidence: analysis.language.confidence,
            summary: analysis.summary.short.clone(),
            keywords: analysis
                .keywords
                .into_iter()
                .map(|(k, _)| k)
                .take(10)
                .collect(),
            readability: analysis.readability,
        }))
    }

    /// Generate cross-modal insights
    fn generate_insights(
        structural: &StructuralAnalysis,
        visual: &Option<VisualAnalysis>,
        semantic: &Option<SemanticAnalysis>,
    ) -> Vec<CrossModalInsight> {
        let mut insights = Vec::new();

        // Content-Structure alignment check
        if let Some(sem) = semantic {
            if structural.dom_node_count > 1000 && sem.content_type == ContentType::Article {
                insights.push(CrossModalInsight {
                    insight_type: InsightType::ContentStructureAlignment,
                    description:
                        "Complex DOM structure for article content - may impact performance"
                            .to_string(),
                    confidence: 0.75,
                    sources: vec![PerceiverType::Structural, PerceiverType::Semantic],
                });
            }
        }

        // Visual-Semantic consistency check
        if let (Some(vis), Some(sem)) = (visual, semantic) {
            if vis.viewport_utilization < 0.3 && sem.content_type == ContentType::Article {
                insights.push(CrossModalInsight {
                    insight_type: InsightType::VisualSemanticConsistency,
                    description: "Low viewport utilization for content-heavy page".to_string(),
                    confidence: 0.65,
                    sources: vec![PerceiverType::Visual, PerceiverType::Semantic],
                });
            }

            // Readability vs contrast check
            if let Some(readability) = sem.readability {
                if readability < 50.0 && vis.avg_contrast < 3.0 {
                    insights.push(CrossModalInsight {
                        insight_type: InsightType::AccessibilityIssue,
                        description:
                            "Low readability combined with poor contrast - accessibility concern"
                                .to_string(),
                        confidence: 0.80,
                        sources: vec![PerceiverType::Visual, PerceiverType::Semantic],
                    });
                }
            }
        }

        // Performance insight from structure
        if structural.dom_node_count > 2000 {
            insights.push(CrossModalInsight {
                insight_type: InsightType::Performance,
                description: format!(
                    "Large DOM tree ({} nodes) may impact rendering performance",
                    structural.dom_node_count
                ),
                confidence: 0.70,
                sources: vec![PerceiverType::Structural],
            });
        }

        insights
    }

    /// Calculate overall confidence score
    fn calculate_confidence(
        structural: &StructuralAnalysis,
        visual: &Option<VisualAnalysis>,
        semantic: &Option<SemanticAnalysis>,
    ) -> f64 {
        let mut confidence = 0.0;
        let mut weight_sum = 0.0;

        // Structural contributes 40%
        if structural.dom_node_count > 0 {
            confidence += 0.9 * 0.4;
            weight_sum += 0.4;
        }

        // Visual contributes 30%
        if visual.is_some() {
            confidence += 0.85 * 0.3;
            weight_sum += 0.3;
        }

        // Semantic contributes 30%
        if let Some(sem) = semantic {
            let sem_confidence = sem.language_confidence * 0.3;
            confidence += sem_confidence;
            weight_sum += 0.3;
        }

        if weight_sum > 0.0 {
            confidence / weight_sum
        } else {
            0.0
        }
    }

    /// Calculate visual complexity score
    fn calculate_visual_complexity(metrics: &VisualMetricsResult) -> f64 {
        // Simple heuristic based on color palette size and contrast
        let color_complexity = (metrics.color_palette.len() as f64 / 10.0).min(1.0);
        let contrast_factor = (metrics.avg_contrast_ratio / 7.0).min(1.0);

        (color_complexity * 0.6 + contrast_factor * 0.4)
            .max(0.0)
            .min(1.0)
    }
}

#[async_trait]
impl PerceptionHub for PerceptionHubImpl {
    async fn perceive(
        &self,
        route: &ExecRoute,
        options: PerceptionOptions,
    ) -> Result<MultiModalPerception> {
        let PerceptionOptions {
            enable_structural,
            enable_visual,
            enable_semantic,
            enable_insights,
            capture_screenshot,
            extract_text,
            timeout_secs,
        } = options;

        let timeout = Duration::from_secs(timeout_secs);

        // Execute requested perceivers with a shared timeout.
        let (structural, visual, semantic) = tokio::time::timeout(timeout, async {
            tokio::try_join!(
                async {
                    if enable_structural {
                        self.analyze_structural(route).await
                    } else {
                        Ok(StructuralAnalysis::default())
                    }
                },
                async {
                    self.analyze_visual(route, enable_visual && capture_screenshot)
                        .await
                },
                async {
                    self.analyze_semantic(route, enable_semantic && extract_text)
                        .await
                }
            )
        })
        .await
        .map_err(|_| HubError::Timeout(format!("Analysis timeout after {}s", timeout_secs)))??;

        // Generate insights
        let insights = if enable_insights {
            Self::generate_insights(&structural, &visual, &semantic)
        } else {
            Vec::new()
        };

        // Calculate confidence
        let confidence = Self::calculate_confidence(&structural, &visual, &semantic);

        Ok(MultiModalPerception {
            structural,
            visual,
            semantic,
            insights,
            confidence,
        })
    }

    fn structural(&self) -> Arc<dyn StructuralPerceiver> {
        self.structural_perceiver.clone()
    }

    fn visual(&self) -> Option<Arc<dyn VisualPerceiver>> {
        self.visual_perceiver
            .as_ref()
            .map(|v| v.clone() as Arc<dyn VisualPerceiver>)
    }

    fn semantic(&self) -> Option<Arc<dyn SemanticPerceiver>> {
        self.semantic_perceiver
            .as_ref()
            .map(|s| s.clone() as Arc<dyn SemanticPerceiver>)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use perceiver_semantic::errors::SemanticError;
    use perceiver_semantic::models::{
        ContentSummary, ContentType, ExtractedText, LanguageInfo, PageIntent,
        SemanticAnalysisResult,
    };
    use perceiver_semantic::{SemanticOptions, SemanticPerceiver, TextExtractionOptions};
    use perceiver_structural::errors::PerceiverError;
    use perceiver_structural::model::{
        AnchorDescriptor, AnchorResolution, DiffFocus, DomAxDiff, DomAxSnapshot, InteractionAdvice,
        JudgeReport, ResolveHint, ResolveOpt, Scope, SelectorOrHint, SnapLevel,
    };
    use perceiver_structural::policy::ResolveOptions;
    use perceiver_visual::errors::VisualError;
    use perceiver_visual::models::{
        CaptureMode, DiffOptions, ImageFormat, Screenshot, ScreenshotOptions, VisualDiffResult,
        VisualMetricsResult,
    };
    use perceiver_visual::VisualPerceiver;
    use soulbrowser_core_types::{FrameId, PageId, SessionId};
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::SystemTime;

    type StructuralResult<T> = std::result::Result<T, PerceiverError>;

    #[test]
    fn test_calculate_confidence() {
        let structural = StructuralAnalysis {
            snapshot_id: "test".to_string(),
            dom_node_count: 100,
            interactive_element_count: 10,
            has_forms: false,
            has_navigation: true,
        };

        let confidence = PerceptionHubImpl::calculate_confidence(&structural, &None, &None);
        assert!(confidence > 0.0);
        assert!(confidence <= 1.0);
    }

    #[test]
    fn test_insight_generation() {
        let structural = StructuralAnalysis {
            snapshot_id: "test".to_string(),
            dom_node_count: 3000,
            interactive_element_count: 50,
            has_forms: true,
            has_navigation: true,
        };

        let insights = PerceptionHubImpl::generate_insights(&structural, &None, &None);

        // Should generate performance insight for large DOM
        assert!(!insights.is_empty());
        assert!(insights
            .iter()
            .any(|i| i.insight_type == InsightType::Performance));
    }

    struct TrackingStructuralPerceiver {
        calls: Arc<AtomicUsize>,
    }

    impl TrackingStructuralPerceiver {
        fn new(calls: Arc<AtomicUsize>) -> Self {
            Self { calls }
        }

        fn panic(&self, method: &str) -> ! {
            panic!("{} unexpectedly invoked in test", method)
        }
    }

    #[async_trait]
    impl StructuralPerceiver for TrackingStructuralPerceiver {
        async fn resolve_anchor(
            &self,
            _route: ExecRoute,
            _hint: ResolveHint,
            _options: ResolveOptions,
        ) -> StructuralResult<AnchorResolution> {
            self.panic("resolve_anchor")
        }

        async fn resolve_anchor_ext(
            &self,
            _route: ExecRoute,
            _hint: SelectorOrHint,
            _options: ResolveOpt,
        ) -> StructuralResult<AnchorResolution> {
            self.panic("resolve_anchor_ext")
        }

        async fn is_visible(
            &self,
            _route: ExecRoute,
            _anchor: &mut AnchorDescriptor,
        ) -> StructuralResult<JudgeReport> {
            self.panic("is_visible")
        }

        async fn is_clickable(
            &self,
            _route: ExecRoute,
            _anchor: &mut AnchorDescriptor,
        ) -> StructuralResult<JudgeReport> {
            self.panic("is_clickable")
        }

        async fn is_enabled(
            &self,
            _route: ExecRoute,
            _anchor: &mut AnchorDescriptor,
        ) -> StructuralResult<JudgeReport> {
            self.panic("is_enabled")
        }

        async fn snapshot_dom_ax(&self, _route: ExecRoute) -> StructuralResult<DomAxSnapshot> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.panic("snapshot_dom_ax")
        }

        async fn snapshot_dom_ax_ext(
            &self,
            _route: ExecRoute,
            _scope: Scope,
            _level: SnapLevel,
        ) -> StructuralResult<DomAxSnapshot> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.panic("snapshot_dom_ax_ext")
        }

        async fn diff_dom_ax(
            &self,
            _route: ExecRoute,
            _base: &DomAxSnapshot,
            _current: &DomAxSnapshot,
        ) -> StructuralResult<DomAxDiff> {
            self.panic("diff_dom_ax")
        }

        async fn diff_dom_ax_ext(
            &self,
            _route: ExecRoute,
            _base: &DomAxSnapshot,
            _current: &DomAxSnapshot,
            _focus: Option<DiffFocus>,
        ) -> StructuralResult<DomAxDiff> {
            self.panic("diff_dom_ax_ext")
        }

        fn advice_for_interaction(&self, _anchor: &AnchorDescriptor) -> Option<InteractionAdvice> {
            None
        }
    }

    struct StubVisualPerceiver;

    #[async_trait]
    impl VisualPerceiver for StubVisualPerceiver {
        async fn capture_screenshot(
            &self,
            route: &ExecRoute,
            _options: ScreenshotOptions,
        ) -> std::result::Result<Screenshot, VisualError> {
            Ok(Screenshot {
                id: "stub-shot".to_string(),
                data: Vec::new(),
                format: ImageFormat::Png,
                width: 10,
                height: 10,
                timestamp: SystemTime::now(),
                page_id: route.page.0.clone(),
                capture_mode: CaptureMode::Viewport,
                clip: None,
            })
        }

        async fn compute_diff(
            &self,
            _before: &Screenshot,
            _after: &Screenshot,
            _options: DiffOptions,
        ) -> std::result::Result<VisualDiffResult, VisualError> {
            Err(VisualError::DiffFailed("not implemented".into()))
        }

        async fn analyze_metrics(
            &self,
            _screenshot: &Screenshot,
        ) -> std::result::Result<VisualMetricsResult, VisualError> {
            Ok(VisualMetricsResult {
                color_palette: vec![(255, 255, 255)],
                avg_contrast_ratio: 1.0,
                layout_stability: 1.0,
                viewport_utilization: 0.5,
            })
        }
    }

    struct StubSemanticPerceiver;

    #[async_trait]
    impl SemanticPerceiver for StubSemanticPerceiver {
        async fn extract_text(
            &self,
            _route: &ExecRoute,
            _options: TextExtractionOptions,
        ) -> std::result::Result<ExtractedText, SemanticError> {
            Ok(ExtractedText {
                body: String::new(),
                title: None,
                description: None,
                headings: Vec::new(),
                links: Vec::new(),
                char_count: 0,
            })
        }

        async fn analyze(
            &self,
            _route: &ExecRoute,
            _options: SemanticOptions,
        ) -> std::result::Result<SemanticAnalysisResult, SemanticError> {
            self.analyze_text(
                &ExtractedText {
                    body: String::new(),
                    title: None,
                    description: None,
                    headings: Vec::new(),
                    links: Vec::new(),
                    char_count: 0,
                },
                SemanticOptions::default(),
            )
            .await
        }

        async fn analyze_text(
            &self,
            _text: &ExtractedText,
            _options: SemanticOptions,
        ) -> std::result::Result<SemanticAnalysisResult, SemanticError> {
            Ok(SemanticAnalysisResult {
                content_type: ContentType::Unknown,
                intent: PageIntent::Unknown,
                language: LanguageInfo {
                    code: "en".to_string(),
                    name: "English".to_string(),
                    confidence: 1.0,
                },
                summary: ContentSummary {
                    short: String::new(),
                    medium: None,
                    key_points: Vec::new(),
                    word_count: 0,
                },
                entities: Vec::new(),
                keywords: HashMap::new(),
                sentiment: None,
                readability: None,
            })
        }
    }

    #[tokio::test]
    async fn visual_only_modes_skip_structural_snapshot() {
        let calls = Arc::new(AtomicUsize::new(0));
        let structural = Arc::new(TrackingStructuralPerceiver::new(calls.clone()));
        let hub = PerceptionHubImpl::structural_only(structural);

        let route = ExecRoute::new(SessionId::new(), PageId::new(), FrameId::new());
        let opts = PerceptionOptions {
            enable_structural: false,
            enable_visual: false,
            enable_semantic: false,
            enable_insights: false,
            capture_screenshot: false,
            extract_text: false,
            timeout_secs: 5,
        };

        let result = hub.perceive(&route, opts).await.expect("perception result");

        assert_eq!(
            calls.load(Ordering::SeqCst),
            0,
            "structural snapshot should be skipped"
        );
        assert_eq!(result.structural.snapshot_id, "structural-disabled");
    }

    #[tokio::test]
    async fn visual_mode_operates_without_structural() {
        let calls = Arc::new(AtomicUsize::new(0));
        let structural = Arc::new(TrackingStructuralPerceiver::new(calls.clone()));
        let visual = Arc::new(StubVisualPerceiver);
        let semantic = Arc::new(StubSemanticPerceiver);

        let hub = PerceptionHubImpl::new(structural, visual, semantic);
        let route = ExecRoute::new(SessionId::new(), PageId::new(), FrameId::new());

        let opts = PerceptionOptions {
            enable_structural: false,
            enable_visual: true,
            enable_semantic: false,
            enable_insights: false,
            capture_screenshot: true,
            extract_text: false,
            timeout_secs: 5,
        };

        let result = hub.perceive(&route, opts).await.expect("perception result");

        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert!(result.visual.is_some(), "visual analysis should run");
        assert_eq!(result.structural.snapshot_id, "structural-disabled");
    }
}
