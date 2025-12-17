///! Main semantic perceiver implementation
use crate::{
    classifier::Classifier, errors::*, keywords::KeywordExtractor, language::LanguageDetector,
    models::*, summarizer::Summarizer,
};
use async_trait::async_trait;
use perceiver_structural::StructuralPerceiver;
use serde_json::Value;
use soulbrowser_core_types::ExecRoute;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

/// Semantic perceiver trait
#[async_trait]
pub trait SemanticPerceiver: Send + Sync {
    /// Extract text content from page
    async fn extract_text(
        &self,
        route: &ExecRoute,
        options: TextExtractionOptions,
    ) -> Result<ExtractedText>;

    /// Perform full semantic analysis on page
    async fn analyze(
        &self,
        route: &ExecRoute,
        options: SemanticOptions,
    ) -> Result<SemanticAnalysisResult>;

    /// Analyze already extracted text
    async fn analyze_text(
        &self,
        text: &ExtractedText,
        options: SemanticOptions,
    ) -> Result<SemanticAnalysisResult>;
}

/// Semantic perceiver implementation
pub struct SemanticPerceiverImpl {
    structural_perceiver: Arc<dyn StructuralPerceiver>,
    language_detector: LanguageDetector,
    classifier: Classifier,
    summarizer: Summarizer,
    keyword_extractor: KeywordExtractor,
}

impl SemanticPerceiverImpl {
    /// Create new semantic perceiver
    pub fn new(structural_perceiver: Arc<dyn StructuralPerceiver>) -> Self {
        Self {
            structural_perceiver,
            language_detector: LanguageDetector::new(),
            classifier: Classifier::new(),
            summarizer: Summarizer::new(),
            keyword_extractor: KeywordExtractor::new(),
        }
    }

    /// Extract text from structural perceiver DOM
    async fn extract_text_from_dom(
        &self,
        route: &ExecRoute,
        options: &TextExtractionOptions,
    ) -> Result<ExtractedText> {
        // Get DOM snapshot from structural perceiver
        let snapshot = self
            .structural_perceiver
            .snapshot_dom_ax(route.clone())
            .await
            .map_err(|e| SemanticError::StructuralError(format!("{:?}", e)))?;

        let strings = snapshot.dom_raw.get("strings").and_then(Value::as_array);
        let documents = snapshot.dom_raw.get("documents").and_then(Value::as_array);

        let mut body_segments = Vec::new();
        let mut heading_nodes = HashSet::new();
        let mut heading_content: BTreeMap<usize, Vec<String>> = BTreeMap::new();
        let mut link_texts: HashMap<usize, Vec<String>> = HashMap::new();
        let mut link_targets: BTreeMap<usize, String> = BTreeMap::new();
        let mut hidden_nodes = HashSet::new();
        let mut title = None;
        let mut description = None;

        if let Some(documents) = documents {
            for document in documents {
                if let Some(nodes) = document.get("nodes").and_then(Value::as_object) {
                    let node_names = nodes.get("nodeName").and_then(Value::as_array);
                    let node_values = nodes.get("nodeValue").and_then(Value::as_array);
                    let node_types = nodes.get("nodeType").and_then(Value::as_array);
                    let parent_indexes = nodes.get("parentIndex").and_then(Value::as_array);
                    let attributes = nodes.get("attributes").and_then(Value::as_array);
                    let count = node_names.map(|arr| arr.len()).unwrap_or(0);

                    for idx in 0..count {
                        let raw_name = node_names
                            .and_then(|arr| arr.get(idx))
                            .and_then(|value| decode_snapshot_string(strings, value))
                            .unwrap_or_default();
                        let node_name = raw_name.to_ascii_uppercase();
                        let attr_map = attributes
                            .and_then(|arr| arr.get(idx))
                            .map(|entry| collect_attributes(strings, entry))
                            .unwrap_or_default();
                        let is_hidden_node = !options.include_hidden && node_is_hidden(&attr_map);
                        if is_hidden_node {
                            hidden_nodes.insert(idx);
                        }

                        if node_name == "TITLE" && options.include_metadata && title.is_none() {
                            if let Some(text) = decode_entry(strings, node_values, idx) {
                                let trimmed = text.trim();
                                if !trimmed.is_empty() {
                                    title = Some(trimmed.to_string());
                                }
                            }
                        }

                        if node_name == "META" && options.include_metadata && description.is_none()
                        {
                            if let Some(desc) = description_from_meta(&attr_map) {
                                description = Some(desc);
                            }
                        }

                        if node_name == "A" {
                            if let Some(href) = attr_map.get("href").map(|v| v.to_string()) {
                                link_targets.insert(idx, href);
                            }
                            if options.include_aria_labels {
                                if let Some(label) = attr_map.get("aria-label") {
                                    let trimmed = label.trim();
                                    if !trimmed.is_empty() {
                                        link_texts
                                            .entry(idx)
                                            .or_default()
                                            .push(trimmed.to_string());
                                    }
                                }
                            }
                        }

                        if node_name == "IMG" && options.include_alt_text {
                            if let Some(alt) = attr_map.get("alt") {
                                let trimmed = alt.trim();
                                if !trimmed.is_empty() {
                                    body_segments.push(trimmed.to_string());
                                }
                            }
                        }

                        if options.include_aria_labels && node_name != "A" {
                            if let Some(label) = attr_map.get("aria-label") {
                                let trimmed = label.trim();
                                if !trimmed.is_empty() {
                                    body_segments.push(trimmed.to_string());
                                }
                            }
                        }

                        if matches!(node_name.as_str(), "H1" | "H2" | "H3" | "H4" | "H5" | "H6") {
                            heading_nodes.insert(idx);
                            if let Some(text) = decode_entry(strings, node_values, idx) {
                                let trimmed = text.trim();
                                if !trimmed.is_empty() {
                                    heading_content
                                        .entry(idx)
                                        .or_default()
                                        .push(trimmed.to_string());
                                }
                            }
                        }

                        let node_type = node_types
                            .and_then(|arr| arr.get(idx))
                            .and_then(Value::as_i64)
                            .unwrap_or(0);
                        if node_type == 3 {
                            if let Some(text) = decode_entry(strings, node_values, idx) {
                                let trimmed = text.trim();
                                if trimmed.is_empty() {
                                    continue;
                                }
                                if hidden_nodes.contains(&idx) && !options.include_hidden {
                                    continue;
                                }
                                let parent = parent_index(parent_indexes, idx);
                                if !options.include_hidden {
                                    if let Some(parent_idx) = parent {
                                        if hidden_nodes.contains(&parent_idx) {
                                            continue;
                                        }
                                    }
                                }
                                if let Some(parent_idx) = parent {
                                    if heading_nodes.contains(&parent_idx) {
                                        heading_content
                                            .entry(parent_idx)
                                            .or_default()
                                            .push(trimmed.to_string());
                                        continue;
                                    }
                                    if link_targets.contains_key(&parent_idx) {
                                        link_texts
                                            .entry(parent_idx)
                                            .or_default()
                                            .push(trimmed.to_string());
                                        continue;
                                    }
                                }
                                body_segments.push(trimmed.to_string());
                            }
                        }
                    }
                }
            }
        }

        let headings = heading_content
            .into_iter()
            .filter_map(|(_idx, parts)| {
                let text = parts.join(" ").trim().to_string();
                if text.is_empty() {
                    None
                } else {
                    Some(text)
                }
            })
            .collect();

        let mut links = Vec::new();
        for (idx, href) in link_targets {
            let text = link_texts
                .get(&idx)
                .map(|parts| parts.join(" ").trim().to_string())
                .filter(|t| !t.is_empty())
                .unwrap_or_else(|| href.clone());
            links.push((text, href));
        }

        let body = body_segments.join(" ");
        let char_count = body.len();

        Ok(ExtractedText {
            body,
            title,
            description,
            headings,
            links,
            char_count,
        })
    }
}

#[async_trait]
impl SemanticPerceiver for SemanticPerceiverImpl {
    async fn extract_text(
        &self,
        route: &ExecRoute,
        options: TextExtractionOptions,
    ) -> Result<ExtractedText> {
        self.extract_text_from_dom(route, &options).await
    }

    async fn analyze(
        &self,
        route: &ExecRoute,
        options: SemanticOptions,
    ) -> Result<SemanticAnalysisResult> {
        // Extract text first
        let text = self
            .extract_text(route, TextExtractionOptions::default())
            .await?;

        // Analyze the extracted text
        self.analyze_text(&text, options).await
    }

    async fn analyze_text(
        &self,
        text: &ExtractedText,
        options: SemanticOptions,
    ) -> Result<SemanticAnalysisResult> {
        // Run analysis components in parallel
        let all_text = text.all_text();

        // Language detection
        let language = self.language_detector.detect(&all_text)?;

        // Content classification (run in blocking task)
        let text_clone = text.clone();
        let classifier = self.classifier.clone();
        let content_type =
            tokio::task::spawn_blocking(move || classifier.classify_content_type(&text_clone))
                .await
                .map_err(|e| SemanticError::AnalysisFailed(format!("Task join error: {}", e)))??;

        // Intent classification
        let text_clone = text.clone();
        let classifier = self.classifier.clone();
        let intent = tokio::task::spawn_blocking(move || classifier.classify_intent(&text_clone))
            .await
            .map_err(|e| SemanticError::AnalysisFailed(format!("Task join error: {}", e)))??;

        // Summarization
        let text_clone = text.clone();
        let summarizer = self.summarizer.clone();
        let summary = tokio::task::spawn_blocking(move || summarizer.summarize(&text_clone))
            .await
            .map_err(|e| SemanticError::SummarizationFailed(format!("Task join error: {}", e)))??;

        // Keyword extraction
        let keywords = if options.extract_keywords {
            let text_clone = text.clone();
            let options_clone = options.clone();
            let extractor = self.keyword_extractor.clone();
            tokio::task::spawn_blocking(move || extractor.extract(&text_clone, &options_clone))
                .await
                .map_err(|e| SemanticError::AnalysisFailed(format!("Task join error: {}", e)))??
        } else {
            std::collections::HashMap::new()
        };

        // Readability analysis
        let readability = if options.analyze_readability {
            let all_text_clone = all_text.clone();
            let summarizer = self.summarizer.clone();
            Some(
                tokio::task::spawn_blocking(move || {
                    summarizer.calculate_readability(&all_text_clone)
                })
                .await
                .map_err(|e| SemanticError::AnalysisFailed(format!("Task join error: {}", e)))?,
            )
        } else {
            None
        };

        let entities = if options.extract_entities {
            let entity_limit = options.max_keywords.max(3).min(16);
            extract_entities_from_text(&all_text, entity_limit)
        } else {
            Vec::new()
        };

        let sentiment = if options.analyze_sentiment {
            Some(sentiment_score(&all_text))
        } else {
            None
        };

        Ok(SemanticAnalysisResult {
            content_type,
            intent,
            language,
            summary,
            entities,
            keywords,
            sentiment,
            readability,
        })
    }
}

fn decode_snapshot_string(strings: Option<&Vec<Value>>, value: &Value) -> Option<String> {
    match value {
        Value::String(s) => Some(s.clone()),
        Value::Number(num) => num.as_u64().and_then(|idx| {
            strings
                .and_then(|table| table.get(idx as usize))
                .and_then(|entry| entry.as_str())
                .map(|s| s.to_string())
        }),
        _ => None,
    }
}

fn decode_entry(
    strings: Option<&Vec<Value>>,
    values: Option<&Vec<Value>>,
    idx: usize,
) -> Option<String> {
    values
        .and_then(|arr| arr.get(idx))
        .and_then(|v| decode_snapshot_string(strings, v))
}

fn collect_attributes(strings: Option<&Vec<Value>>, entry: &Value) -> HashMap<String, String> {
    let mut result = HashMap::new();
    if let Some(list) = entry.as_array() {
        let mut iter = list.iter();
        while let Some(name_raw) = iter.next() {
            let value_raw = iter.next();
            if let Some(name) = decode_snapshot_string(strings, name_raw) {
                let value = value_raw
                    .and_then(|raw| decode_snapshot_string(strings, raw))
                    .unwrap_or_default();
                result.insert(name.to_lowercase(), value);
            }
        }
    }
    result
}

fn parent_index(values: Option<&Vec<Value>>, idx: usize) -> Option<usize> {
    values
        .and_then(|arr| arr.get(idx))
        .and_then(Value::as_i64)
        .and_then(|raw| if raw >= 0 { Some(raw as usize) } else { None })
}

fn node_is_hidden(attrs: &HashMap<String, String>) -> bool {
    if attrs.contains_key("hidden") {
        return true;
    }
    if attrs
        .get("aria-hidden")
        .map(|value| value.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
    {
        return true;
    }
    if let Some(style) = attrs.get("style") {
        let style_lower = style.to_ascii_lowercase();
        return style_lower.contains("display:none") || style_lower.contains("visibility:hidden");
    }
    false
}

fn description_from_meta(attrs: &HashMap<String, String>) -> Option<String> {
    let content = attrs
        .get("content")
        .map(|v| v.trim())
        .filter(|v| !v.is_empty());
    let candidate = content?;
    let name = attrs.get("name").map(|v| v.to_ascii_lowercase());
    let property = attrs.get("property").map(|v| v.to_ascii_lowercase());
    let matches_name = name.as_deref() == Some("description");
    let matches_property = matches!(
        property.as_deref(),
        Some("og:description") | Some("twitter:description")
    );
    if matches_name || matches_property {
        Some(candidate.to_string())
    } else {
        None
    }
}

fn extract_entities_from_text(text: &str, limit: usize) -> Vec<Entity> {
    let mut seen = HashSet::new();
    let mut entities = Vec::new();

    for token in text.split(|c: char| !c.is_alphabetic()) {
        if token.len() < 3 {
            continue;
        }
        let first = token.chars().next().unwrap_or_default();
        if !first.is_uppercase() {
            continue;
        }
        let normalized = token.trim_matches(|c: char| !c.is_alphanumeric());
        if normalized.is_empty() {
            continue;
        }
        let key = normalized.to_lowercase();
        if seen.insert(key) {
            entities.push(Entity {
                text: normalized.to_string(),
                entity_type: guess_entity_type(normalized),
                confidence: 0.55,
            });
            if entities.len() >= limit {
                break;
            }
        }
    }

    entities
}

fn guess_entity_type(token: &str) -> String {
    let lower = token.to_ascii_lowercase();
    if lower.ends_with("inc")
        || lower.ends_with("corp")
        || lower.ends_with("co")
        || lower.ends_with("ltd")
    {
        "organization".to_string()
    } else if lower.ends_with("city")
        || lower.ends_with("town")
        || lower.ends_with("bay")
        || lower.ends_with("lake")
    {
        "location".to_string()
    } else if lower.chars().all(|c| c.is_uppercase()) {
        "acronym".to_string()
    } else {
        "person".to_string()
    }
}

fn sentiment_score(text: &str) -> f64 {
    const POSITIVE: [&str; 12] = [
        "excellent",
        "good",
        "great",
        "positive",
        "fast",
        "love",
        "clean",
        "win",
        "success",
        "happy",
        "delight",
        "secure",
    ];
    const NEGATIVE: [&str; 12] = [
        "bad", "slow", "negative", "error", "fail", "broken", "bug", "sad", "issue", "poor",
        "crash", "risk",
    ];

    let mut positive = 0f64;
    let mut negative = 0f64;
    for word in text.split(|c: char| !c.is_alphabetic()) {
        if word.is_empty() {
            continue;
        }
        let lower = word.to_ascii_lowercase();
        if POSITIVE.contains(&lower.as_str()) {
            positive += 1.0;
        }
        if NEGATIVE.contains(&lower.as_str()) {
            negative += 1.0;
        }
    }

    if positive + negative == 0.0 {
        0.0
    } else {
        ((positive - negative) / (positive + negative)).clamp(-1.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use perceiver_structural::errors::PerceiverError;
    use perceiver_structural::policy::ResolveOptions;
    use perceiver_structural::{
        AnchorDescriptor as StructuralAnchor, AnchorResolution, DiffFocus, DomAxDiff,
        DomAxSnapshot, InteractionAdvice, JudgeReport, ResolveHint, ResolveOpt, Scope,
        SelectorOrHint, SnapLevel, StructuralPerceiver,
    };
    use serde_json::json;
    use soulbrowser_core_types::{ExecRoute, FrameId, PageId, SessionId};
    use std::sync::Arc;

    type StructuralResult<T> = std::result::Result<T, PerceiverError>;

    struct StaticStructuralPerceiver {
        snapshot: DomAxSnapshot,
    }

    impl StaticStructuralPerceiver {
        fn new(snapshot: DomAxSnapshot) -> Self {
            Self { snapshot }
        }

        fn unsupported<T>(&self, method: &str) -> StructuralResult<T> {
            Err(PerceiverError::internal(format!("{method} not supported")))
        }
    }

    #[async_trait]
    impl StructuralPerceiver for StaticStructuralPerceiver {
        async fn resolve_anchor(
            &self,
            _route: ExecRoute,
            _hint: ResolveHint,
            _options: ResolveOptions,
        ) -> StructuralResult<AnchorResolution> {
            self.unsupported("resolve_anchor")
        }

        async fn resolve_anchor_ext(
            &self,
            _route: ExecRoute,
            _hint: SelectorOrHint,
            _options: ResolveOpt,
        ) -> StructuralResult<AnchorResolution> {
            self.unsupported("resolve_anchor_ext")
        }

        async fn is_visible(
            &self,
            _route: ExecRoute,
            _anchor: &mut StructuralAnchor,
        ) -> StructuralResult<JudgeReport> {
            self.unsupported("is_visible")
        }

        async fn is_clickable(
            &self,
            _route: ExecRoute,
            _anchor: &mut StructuralAnchor,
        ) -> StructuralResult<JudgeReport> {
            self.unsupported("is_clickable")
        }

        async fn is_enabled(
            &self,
            _route: ExecRoute,
            _anchor: &mut StructuralAnchor,
        ) -> StructuralResult<JudgeReport> {
            self.unsupported("is_enabled")
        }

        async fn snapshot_dom_ax(&self, _route: ExecRoute) -> StructuralResult<DomAxSnapshot> {
            Ok(self.snapshot.clone())
        }

        async fn snapshot_dom_ax_ext(
            &self,
            _route: ExecRoute,
            _scope: Scope,
            _level: SnapLevel,
        ) -> StructuralResult<DomAxSnapshot> {
            Ok(self.snapshot.clone())
        }

        async fn diff_dom_ax(
            &self,
            _route: ExecRoute,
            _base: &DomAxSnapshot,
            _current: &DomAxSnapshot,
        ) -> StructuralResult<DomAxDiff> {
            self.unsupported("diff_dom_ax")
        }

        async fn diff_dom_ax_ext(
            &self,
            _route: ExecRoute,
            _base: &DomAxSnapshot,
            _current: &DomAxSnapshot,
            _focus: Option<DiffFocus>,
        ) -> StructuralResult<DomAxDiff> {
            self.unsupported("diff_dom_ax_ext")
        }

        fn advice_for_interaction(&self, _anchor: &StructuralAnchor) -> Option<InteractionAdvice> {
            None
        }
    }

    fn build_route() -> ExecRoute {
        ExecRoute::new(SessionId::new(), PageId::new(), FrameId::new())
    }

    fn build_dom_snapshot(route: &ExecRoute) -> DomAxSnapshot {
        let nodes = json!({
            "nodeName": [
                "DOCUMENT",
                "HTML",
                "HEAD",
                "TITLE",
                "#text",
                "META",
                "BODY",
                "H1",
                "#text",
                "A",
                "#text",
                "#text"
            ],
            "nodeValue": [
                Value::Null,
                Value::Null,
                Value::Null,
                Value::String("Example Title".into()),
                Value::Null,
                Value::Null,
                Value::Null,
                Value::Null,
                Value::String("Welcome".into()),
                Value::Null,
                Value::String("Home".into()),
                Value::String("Body copy".into())
            ],
            "nodeType": [9, 1, 1, 1, 3, 1, 1, 1, 3, 1, 3, 3],
            "parentIndex": [
                -1,
                0,
                1,
                2,
                3,
                2,
                1,
                6,
                7,
                6,
                9,
                6
            ],
            "attributes": [
                [],
                [],
                [],
                [],
                [],
                ["name", "description", "content", "Meta Description"],
                [],
                [],
                [],
                ["href", "https://example.com", "aria-label", "Visit"],
                [],
                []
            ]
        });

        DomAxSnapshot::new(
            route.page.clone(),
            route.frame.clone(),
            Some(route.session.clone()),
            SnapLevel::Full,
            json!({ "documents": [{ "nodes": nodes }], "strings": [] }),
            json!({}),
        )
    }

    fn build_perceiver() -> (SemanticPerceiverImpl, ExecRoute) {
        let route = build_route();
        let snapshot = build_dom_snapshot(&route);
        let structural = Arc::new(StaticStructuralPerceiver::new(snapshot));
        (SemanticPerceiverImpl::new(structural), route)
    }

    #[tokio::test]
    async fn test_analyze_text() {
        // Create mock structural perceiver
        // This test would need proper mocking infrastructure
        // For now, we test the components individually in their own modules
    }

    #[tokio::test]
    async fn extract_text_reports_metadata() {
        let (perceiver, route) = build_perceiver();
        let result = perceiver
            .extract_text(&route, TextExtractionOptions::default())
            .await
            .expect("text extraction");

        assert_eq!(result.title.as_deref(), Some("Example Title"));
        assert_eq!(result.description.as_deref(), Some("Meta Description"));
        assert!(result
            .headings
            .iter()
            .any(|heading| heading.contains("Welcome")));
        assert!(result
            .links
            .iter()
            .any(|(text, href)| { text.contains("Home") && href.contains("example.com") }));
    }

    #[tokio::test]
    async fn semantic_options_enable_entities_and_sentiment() {
        let (perceiver, _route) = build_perceiver();
        let sample = ExtractedText {
            body: "OpenAI built a great product that avoids bad bugs".to_string(),
            title: Some("Amazing Launch".to_string()),
            description: None,
            headings: vec!["Hero".to_string()],
            links: Vec::new(),
            char_count: 64,
        };

        let mut options = SemanticOptions::default();
        options.extract_entities = true;
        options.analyze_sentiment = true;

        let enriched = perceiver
            .analyze_text(&sample, options.clone())
            .await
            .expect("analysis");

        assert!(!enriched.entities.is_empty());
        assert!(enriched.sentiment.is_some());

        options.extract_entities = false;
        options.analyze_sentiment = false;
        options.extract_keywords = false;
        options.analyze_readability = false;

        let minimal = perceiver
            .analyze_text(&sample, options)
            .await
            .expect("analysis");

        assert!(minimal.entities.is_empty());
        assert!(minimal.sentiment.is_none());
    }

    #[test]
    fn entities_are_inferred_from_capitalized_words() {
        let sample = "Welcome to OpenAI Research in San Francisco";
        let entities = extract_entities_from_text(sample, 5);
        assert!(entities.iter().any(|e| e.text == "OpenAI"));
        assert!(entities.iter().any(|e| e.text == "San"));
    }

    #[test]
    fn sentiment_helper_detects_polarity() {
        let positive = sentiment_score("great win and excellent, happy users");
        assert!(positive > 0.0);
        let negative = sentiment_score("bad bug caused slow error and poor ux");
        assert!(negative < 0.0);
    }
}
