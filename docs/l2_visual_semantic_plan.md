# L2 Visual & Semantic Perceiver Development Plan (2025-10)

> Goal: Implement Visual and Semantic perception layers to complement the existing Structural Perceiver, enabling multi-modal page understanding for browser automation.

## 0. Current Status

**Completed:**
- ✅ **Structural Perceiver**: Production-ready with DOM/AX analysis, caching, metrics
- ✅ **L0 CDP Adapter**: Screenshot capability exists via `Page.captureScreenshot`

**Pending:**
- ⏳ **Visual Perceiver**: Screenshot capture, OCR, visual diff, element detection
- ⏳ **Semantic Perceiver**: Content understanding, text analysis, intent extraction

## 1. Architecture Overview

```
L2: Layered Perception
├── perceiver-structural/  ✅ DONE
│   ├── DOM snapshot & AX tree analysis
│   ├── Anchor resolution with caching
│   └── Structural diff computation
│
├── perceiver-visual/      🔄 NEW
│   ├── Screenshot capture & management
│   ├── OCR text extraction (tesseract-rs)
│   ├── Visual diff (image comparison)
│   ├── Element bounding box detection
│   └── Visual metrics (color, contrast, layout)
│
└── perceiver-semantic/    🔄 NEW
    ├── Content classification
    ├── Text summarization
    ├── Intent extraction
    ├── Sentiment analysis
    └── Language detection
```

## 2. Phase Breakdown

### Phase 1: Visual Perceiver Core (Week 1-2)

**Objective**: Implement screenshot capture, storage, and basic visual analysis

#### 1.1 Crate Structure Setup
- Create `crates/perceiver-visual/` with standard Rust crate layout
- Define core traits: `VisualPerceiver`, `VisualPort`, `VisualSnapshot`
- Establish data models: `Screenshot`, `BoundingBox`, `VisualMetrics`

#### 1.2 Screenshot Capture
- Integrate with CDP `Page.captureScreenshot` via `CdpAdapter`
- Support formats: PNG (primary), JPEG (compressed)
- Implement viewport vs full-page capture modes
- Add screenshot caching with TTL (similar to structural perceiver)

#### 1.3 OCR Integration
- Add `tesseract-rs` dependency for text extraction
- Implement text extraction from screenshots
- Support language detection and multi-language OCR
- Cache OCR results by screenshot hash

#### 1.4 Visual Diff
- Implement pixel-wise image comparison
- Calculate diff metrics: pixel difference %, structural similarity
- Generate visual diff images highlighting changes
- Support threshold-based change detection

### Phase 2: Visual Perceiver Advanced (Week 3)

#### 2.1 Element Visual Detection
- Detect clickable elements via visual heuristics (buttons, links)
- Calculate element visibility (occlusion, opacity, viewport)
- Extract visual properties (color, size, position)
- Integrate with structural perceiver anchors

#### 2.2 Visual Metrics & Analysis
- Color palette extraction
- Contrast ratio calculation (WCAG compliance)
- Layout stability metrics (CLS-like)
- Responsive design validation

#### 2.3 Performance & Optimization
- Screenshot compression and storage optimization
- Parallel OCR processing
- LRU cache for frequently accessed images
- Lazy loading for large screenshots

### Phase 3: Semantic Perceiver Core (Week 4-5)

**Objective**: Implement content understanding and text analysis

#### 3.1 Crate Structure Setup
- Create `crates/perceiver-semantic/` with standard layout
- Define core traits: `SemanticPerceiver`, `SemanticPort`
- Establish data models: `ContentAnalysis`, `Intent`, `Summary`

#### 3.2 Text Analysis
- Content classification (article, form, navigation, e-commerce)
- Main content extraction (remove boilerplate)
- Heading structure analysis
- Keyword extraction

#### 3.3 Language Processing
- Language detection (lingua-rs)
- Text summarization (extractive)
- Named entity recognition (basic)
- Sentiment analysis (optional)

#### 3.4 Intent Extraction
- Form intent detection (login, search, checkout)
- Action affordance detection (what can user do?)
- Content purpose classification
- User journey mapping

### Phase 4: Integration & Testing (Week 6)

#### 4.1 L0/L1 Integration
- Wire visual/semantic perceivers to CDP adapter
- Integrate with State Center for event logging
- Add policy controls for resource usage
- Coordinate with scheduler for concurrent operations

#### 4.2 Multi-Modal Perception
- Create unified `PerceptionHub` coordinating all three perceivers
- Implement cross-perceiver data fusion
- Support multi-modal element resolution (structure + visual + semantic)
- Add confidence scoring across modalities

#### 4.3 CLI & Observability
- Add `soulbrowser visual` command (screenshot, OCR, diff)
- Add `soulbrowser semantic` command (analyze content, extract intent)
- Extend `soulbrowser perceiver` for multi-modal stats
- Add metrics to State Center dashboard

#### 4.4 Testing & Validation
- Unit tests for visual/semantic core functions
- Integration tests with real Chrome (opt-in)
- Benchmark performance (latency, memory usage)
- Visual regression test suite

### Phase 5: Documentation & Polish (Week 7)

#### 5.1 Documentation
- API documentation for both crates
- Usage examples and tutorials
- Configuration guide (policies, thresholds)
- Troubleshooting guide (OCR failures, performance)

#### 5.2 Performance Tuning
- Optimize screenshot compression
- Parallelize OCR processing
- Cache warm-up strategies
- Resource usage profiling

#### 5.3 Final Integration
- Update README with new capabilities
- Update architecture diagrams
- Create migration guide for existing users
- Update roadmap status

## 3. Technical Specifications

### Visual Perceiver API

```rust
#[async_trait]
pub trait VisualPerceiver {
    /// Capture screenshot of current viewport or full page
    async fn capture_screenshot(
        &self,
        route: &ExecRoute,
        options: ScreenshotOptions,
    ) -> Result<Screenshot, VisualError>;

    /// Extract text from screenshot using OCR
    async fn extract_text(
        &self,
        screenshot: &Screenshot,
        options: OcrOptions,
    ) -> Result<OcrResult, VisualError>;

    /// Compute visual diff between two screenshots
    async fn compute_diff(
        &self,
        before: &Screenshot,
        after: &Screenshot,
        options: DiffOptions,
    ) -> Result<VisualDiff, VisualError>;

    /// Detect visually prominent elements
    async fn detect_elements(
        &self,
        screenshot: &Screenshot,
        options: DetectionOptions,
    ) -> Result<Vec<VisualElement>, VisualError>;

    /// Calculate visual metrics (color, contrast, layout)
    async fn analyze_metrics(
        &self,
        screenshot: &Screenshot,
    ) -> Result<VisualMetrics, VisualError>;
}

pub struct ScreenshotOptions {
    pub format: ImageFormat,        // PNG | JPEG
    pub quality: u8,                 // 0-100
    pub full_page: bool,             // viewport vs full page
    pub clip: Option<BoundingBox>,   // crop region
}

pub struct OcrOptions {
    pub language: String,            // "eng", "chi_sim", etc.
    pub page_segmentation: PsmMode,  // single_block, auto, etc.
    pub whitelist: Option<String>,   // character whitelist
}

pub struct VisualDiff {
    pub pixel_diff_percent: f64,    // 0.0-100.0
    pub structural_similarity: f64,  // 0.0-1.0
    pub diff_image: Option<Vec<u8>>, // highlighted diff
    pub changed_regions: Vec<BoundingBox>,
}
```

### Semantic Perceiver API

```rust
#[async_trait]
pub trait SemanticPerceiver {
    /// Classify page content type
    async fn classify_content(
        &self,
        route: &ExecRoute,
        options: ClassifyOptions,
    ) -> Result<ContentClassification, SemanticError>;

    /// Extract main content and summarize
    async fn summarize_content(
        &self,
        route: &ExecRoute,
        options: SummaryOptions,
    ) -> Result<ContentSummary, SemanticError>;

    /// Extract user intents and affordances
    async fn extract_intents(
        &self,
        route: &ExecRoute,
    ) -> Result<Vec<Intent>, SemanticError>;

    /// Analyze text sentiment and tone
    async fn analyze_sentiment(
        &self,
        text: &str,
    ) -> Result<SentimentAnalysis, SemanticError>;

    /// Detect language and extract metadata
    async fn detect_language(
        &self,
        text: &str,
    ) -> Result<LanguageInfo, SemanticError>;
}

pub struct ContentClassification {
    pub primary_type: ContentType,   // Article, Form, Product, etc.
    pub confidence: f64,              // 0.0-1.0
    pub secondary_types: Vec<ContentType>,
    pub features: Vec<String>,        // detected features
}

pub struct ContentSummary {
    pub main_text: String,            // extracted main content
    pub summary: String,              // brief summary
    pub headings: Vec<Heading>,       // structured headings
    pub keywords: Vec<String>,        // extracted keywords
    pub word_count: usize,
}

pub struct Intent {
    pub action: IntentAction,         // Login, Search, Purchase, etc.
    pub confidence: f64,              // 0.0-1.0
    pub elements: Vec<IntentElement>, // associated elements
    pub description: String,
}

pub enum ContentType {
    Article,
    ProductPage,
    SearchResults,
    Form,
    Navigation,
    Dashboard,
    Authentication,
    Checkout,
    Unknown,
}

pub enum IntentAction {
    Login,
    Signup,
    Search,
    Filter,
    Purchase,
    Submit,
    Navigate,
    Read,
}
```

## 4. Dependencies

### Visual Perceiver Dependencies
```toml
[dependencies]
# Core
tokio = { version = "1", features = ["full"] }
async-trait = "0.1"
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"

# Image processing
image = "0.25"                    # Image manipulation
tesseract-rs = "0.1"              # OCR engine bindings
imageproc = "0.25"                # Image processing algorithms

# Caching & storage
dashmap = "6.0"                   # Concurrent caching
lru = "0.12"                      # LRU cache for images

# Integration
cdp-adapter = { path = "../cdp-adapter" }
soulbrowser-core-types = { path = "../core-types" }
```

### Semantic Perceiver Dependencies
```toml
[dependencies]
# Core
tokio = { version = "1", features = ["full"] }
async-trait = "0.1"
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"

# NLP & text processing
lingua = "1.6"                    # Language detection
unicode-segmentation = "1.11"     # Text segmentation
regex = "1.10"                    # Pattern matching

# Optional ML libraries (future)
# onnxruntime = "0.0.14"         # ONNX runtime for ML models
# tokenizers = "0.19"             # Text tokenization

# Integration
cdp-adapter = { path = "../cdp-adapter" }
perceiver-structural = { path = "../perceiver-structural" }
soulbrowser-core-types = { path = "../core-types" }
```

## 5. Risks & Mitigations

### Visual Perceiver Risks
- **OCR Accuracy**: Tesseract may struggle with complex layouts
  - *Mitigation*: Combine with structural text extraction, use preprocessing
- **Performance**: Screenshot capture and OCR are CPU-intensive
  - *Mitigation*: Parallel processing, aggressive caching, quality/speed tradeoffs
- **Memory Usage**: Large screenshots consume significant memory
  - *Mitigation*: Compression, LRU eviction, streaming processing

### Semantic Perceiver Risks
- **Accuracy**: Heuristic-based classification may be imprecise
  - *Mitigation*: Start simple, iterate with real-world pages, confidence scoring
- **Language Support**: Limited to languages supported by libraries
  - *Mitigation*: Focus on English first, add languages incrementally
- **Performance**: NLP operations can be slow on large documents
  - *Mitigation*: Content length limits, sampling, parallel processing

## 6. Success Criteria

### Visual Perceiver MVP
- ✅ Screenshot capture working with CDP adapter
- ✅ OCR text extraction with ≥80% accuracy on common pages
- ✅ Visual diff detection with configurable thresholds
- ✅ Element detection for clickable items
- ✅ <500ms p95 latency for screenshot capture
- ✅ <2s p95 latency for OCR on full page

### Semantic Perceiver MVP
- ✅ Content classification with ≥70% accuracy
- ✅ Main content extraction removing boilerplate
- ✅ Intent detection for common patterns (login, search, checkout)
- ✅ Language detection supporting top 10 languages
- ✅ <200ms p95 latency for classification
- ✅ <500ms p95 latency for content summarization

### Integration Success
- ✅ CLI commands `soulbrowser visual` and `soulbrowser semantic` functional
- ✅ Multi-modal perception hub coordinating all three perceivers
- ✅ Integration tests passing with real Chrome
- ✅ Documentation complete with examples

## 7. Timeline Estimate

| Phase | Duration | Deliverables |
|-------|----------|--------------|
| Phase 1: Visual Core | 2 weeks | Screenshot, OCR, basic diff |
| Phase 2: Visual Advanced | 1 week | Element detection, metrics |
| Phase 3: Semantic Core | 2 weeks | Classification, summarization, intent |
| Phase 4: Integration | 1 week | Multi-modal hub, CLI, tests |
| Phase 5: Documentation | 1 week | Docs, polish, benchmarks |
| **Total** | **7 weeks** | **Production-ready Visual & Semantic Perceivers** |

## 8. Next Actions

1. **Immediate** (Week 1):
   - ✅ Create this development plan document
   - Create `perceiver-visual` crate structure
   - Implement screenshot capture via CDP
   - Add basic OCR integration
   - Unit tests for core functionality

2. **Short-term** (Week 2-3):
   - Visual diff implementation
   - Element detection
   - Performance optimization
   - Integration tests

3. **Medium-term** (Week 4-6):
   - Semantic perceiver crate
   - Multi-modal integration
   - CLI commands
   - Comprehensive testing

4. **Polish** (Week 7):
   - Documentation
   - Benchmarking
   - README updates
   - Final testing

---

**Status**: 📝 Plan created 2025-10-20
**Next Milestone**: Visual Perceiver crate structure + screenshot capture
