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
        // Create timeout
        let timeout = Duration::from_secs(options.timeout_secs);

        // Run analyses in parallel where possible
        let structural_fut = self.analyze_structural(route);
        let visual_fut =
            self.analyze_visual(route, options.enable_visual && options.capture_screenshot);
        let semantic_fut =
            self.analyze_semantic(route, options.enable_semantic && options.extract_text);

        // Execute with timeout
        let (structural, visual, semantic) = tokio::time::timeout(timeout, async {
            tokio::try_join!(
                async { structural_fut.await },
                async { visual_fut.await },
                async { semantic_fut.await }
            )
        })
        .await
        .map_err(|_| {
            HubError::Timeout(format!("Analysis timeout after {}s", options.timeout_secs))
        })??;

        // Generate insights
        let insights = if options.enable_insights {
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
}
