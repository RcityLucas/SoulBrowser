# L2 Perception Integration Testing

This document describes how to run the L2 multi-modal perception integration tests.

## Prerequisites

1. **Real Chrome/Chromium**: The integration tests require a real Chrome browser
2. **Environment Variable**: Set `SOULBROWSER_USE_REAL_CHROME=1` to enable real browser tests

## Test Suite

The L2 integration test suite (`l2_perception_integration.rs`) includes:

### Individual Perceiver Tests

1. **`test_structural_perception`**
   - Tests DOM/AX tree analysis
   - Verifies node counting and element detection
   - Validates structural metrics

2. **`test_visual_perception`**
   - Tests screenshot capture
   - Validates color detection and analysis
   - Checks contrast and viewport utilization metrics

3. **`test_semantic_perception`**
   - Tests language detection
   - Validates content classification
   - Verifies keyword extraction and summarization

### Integration Tests

4. **`test_multimodal_perception`**
   - Tests all three perceivers working together
   - Validates cross-modal coordination
   - Checks overall confidence scoring

5. **`test_cross_modal_insights`**
   - Tests insight generation from multiple modalities
   - Uses complex page (Wikipedia) to generate insights
   - Validates insight types and confidence levels

6. **`test_perception_timeout`**
   - Tests timeout handling and error recovery
   - Validates graceful degradation

## Running the Tests

### Run All L2 Integration Tests

```bash
export SOULBROWSER_USE_REAL_CHROME=1
cargo test --test l2_perception_integration
```

### Run Individual Test

```bash
export SOULBROWSER_USE_REAL_CHROME=1
cargo test --test l2_perception_integration test_multimodal_perception -- --nocapture
```

### Run with Detailed Output

```bash
export SOULBROWSER_USE_REAL_CHROME=1
RUST_LOG=info cargo test --test l2_perception_integration -- --nocapture
```

## Expected Output

Successful test runs will show:

```
✓ Structural perception: 127 DOM nodes
✓ Visual perception:
  - Screenshot ID: screenshot_abc123
  - Colors: 5
  - Contrast: 4.52
  - Viewport: 78.3%
✓ Semantic perception:
  - Language: en (95.2%)
  - Content: Example
  - Intent: Informational
  - Keywords: ["example", "domain", "illustrative"]
✓ Multi-modal perception:
  - Structural: 127 nodes
  - Visual: present
  - Semantic: present
  - Insights: 2
  - Confidence: 87.5%
✓ Cross-modal insights test:
  - Generated 3 insights
  - [Performance] Large DOM tree (3247 nodes) may impact rendering performance (confidence: 70%)
  - [AccessibilityIssue] Low readability combined with poor contrast (confidence: 80%)
✓ Timeout handling verified

test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured
```

## Test Architecture

Each test follows this pattern:

1. **Setup**: Create CDP adapter and all three perceivers
2. **Navigate**: Load test URL and wait for DOM ready
3. **Perceive**: Execute perception with specific options
4. **Validate**: Assert expected results and metrics
5. **Cleanup**: Shutdown adapter and release resources

## Troubleshooting

### Tests Skipped

If you see "Skipping real Chrome test", the `SOULBROWSER_USE_REAL_CHROME` variable is not set.

```bash
export SOULBROWSER_USE_REAL_CHROME=1
```

### Chrome Not Found

If Chrome executable is not in the standard location:

```bash
export SOULBROWSER_CHROME=/path/to/chrome
export SOULBROWSER_USE_REAL_CHROME=1
cargo test --test l2_perception_integration
```

### Timeout Issues

For slow networks or systems, increase test timeouts by modifying the `timeout_secs` parameter in the test code.

### Headful Mode for Debugging

To see what's happening in the browser, modify test code to disable headless mode:

```rust
let mut config = CdpConfig::default();
config.headless = false;
let adapter = Arc::new(CdpAdapter::new(config, bus));
```

## Performance Benchmarks

Expected test execution times (on average hardware):

- Individual perceiver tests: 2-5 seconds each
- Multi-modal test: 5-10 seconds
- Cross-modal insights test: 10-15 seconds (uses Wikipedia)
- Full suite: ~30-45 seconds

## CI/CD Integration

For continuous integration:

```yaml
# .github/workflows/l2-tests.yml
- name: Run L2 Integration Tests
  env:
    SOULBROWSER_USE_REAL_CHROME: 1
  run: cargo test --test l2_perception_integration
```

Note: Requires Chrome/Chromium installed in CI environment.
