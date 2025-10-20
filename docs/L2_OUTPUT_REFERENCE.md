# L2 感知系统输出信息参考

本文档详细说明 L2 多模态感知系统运行时会输出的所有信息。

## 📊 CLI 命令输出格式

### 完整多模态分析输出

当运行 `soulbrowser perceive --url <URL> --all --insights` 时：

```
🔍 Multi-Modal Perception Analysis
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
URL: https://www.wikipedia.org
Modes: 📊 Structural 👁️  Visual 🧠 Semantic

📊 Structural Analysis
  DOM nodes: 3247
  Interactive elements: 145
  Has forms: true
  Has navigation: true

👁️  Visual Analysis
  Dominant colors: 5 detected
  Avg contrast: 4.52
  Viewport utilization: 78.3%
  Visual complexity: 0.67
  Screenshot saved: output.png

🧠 Semantic Analysis
  Content type: Portal
  Page intent: Navigation
  Language: en (95.2% confidence)
  Readability score: 62.3
  Keywords: wikipedia, encyclopedia, free, knowledge, articles
  Summary: Wikipedia is a free online encyclopedia that anyone can edit.
           It contains millions of articles in multiple languages covering
           various topics from science to history.

💡 Cross-Modal Insights
  • [Performance] Large DOM tree (3247 nodes) may impact rendering performance (confidence: 70%)
  • [AccessibilityIssue] Low readability combined with poor contrast - accessibility concern (confidence: 80%)
  • [ContentStructureAlignment] Complex DOM structure for article content - may impact performance (confidence: 75%)

Overall confidence: 87.5%

📄 Results saved to: results.json
```

## 📋 详细字段说明

### 1. 结构分析 (Structural Analysis)

输出的信息包括：

| 字段 | 类型 | 说明 | 示例值 |
|------|------|------|--------|
| DOM nodes | 整数 | 页面中 DOM 节点总数 | 3247 |
| Interactive elements | 整数 | 可交互元素数量（按钮、链接、输入框等） | 145 |
| Has forms | 布尔 | 页面是否包含表单 | true |
| Has navigation | 布尔 | 页面是否包含导航元素 | true |

**内部数据结构**：
```rust
StructuralAnalysis {
    snapshot_id: String,        // 快照 ID
    dom_node_count: usize,      // DOM 节点数
    interactive_element_count: usize,  // 交互元素数
    has_forms: bool,            // 是否有表单
    has_navigation: bool,       // 是否有导航
}
```

### 2. 视觉分析 (Visual Analysis)

输出的信息包括：

| 字段 | 类型 | 说明 | 示例值 | 范围 |
|------|------|------|--------|------|
| Dominant colors | 整数 | 检测到的主要颜色数量 | 5 | 1-10 |
| Avg contrast | 浮点数 | 平均对比度比率 | 4.52 | 0.0-21.0 |
| Viewport utilization | 百分比 | 视口利用率 | 78.3% | 0-100% |
| Visual complexity | 浮点数 | 视觉复杂度评分 | 0.67 | 0.0-1.0 |
| Screenshot saved | 路径 | 截图保存路径 | output.png | - |

**颜色格式**：RGB 元组 `(r, g, b)`，例如：
```
[(255, 255, 255), (0, 0, 0), (52, 101, 164), (230, 230, 230), (102, 102, 102)]
```

**对比度解释**：
- `< 3.0`: 对比度差，可能有可访问性问题
- `3.0 - 4.5`: 符合 WCAG AA 标准（大文本）
- `4.5 - 7.0`: 符合 WCAG AA 标准（所有文本）
- `> 7.0`: 符合 WCAG AAA 标准

**内部数据结构**：
```rust
VisualAnalysis {
    screenshot_id: String,                    // 截图 ID
    dominant_colors: Vec<(u8, u8, u8)>,      // RGB 颜色列表
    avg_contrast: f64,                        // 平均对比度
    viewport_utilization: f64,                // 视口利用率 [0.0-1.0]
    complexity: f64,                          // 复杂度 [0.0-1.0]
}
```

### 3. 语义分析 (Semantic Analysis)

输出的信息包括：

| 字段 | 类型 | 说明 | 示例值 |
|------|------|------|--------|
| Content type | 枚举 | 内容类型分类 | Portal |
| Page intent | 枚举 | 页面意图 | Navigation |
| Language | 字符串 | 检测到的语言代码 | en |
| Language confidence | 百分比 | 语言检测置信度 | 95.2% |
| Readability score | 浮点数 | Flesch-Kincaid 可读性评分 | 62.3 |
| Keywords | 列表 | 提取的关键词（最多10个） | wikipedia, encyclopedia, free... |
| Summary | 字符串 | 页面内容摘要 | Wikipedia is a free... |

**内容类型 (ContentType)**：
```rust
pub enum ContentType {
    Article,        // 文章/博客
    Portal,         // 门户/主页
    Form,           // 表单页面
    Product,        // 产品页面
    Profile,        // 用户资料
    Search,         // 搜索页面
    Navigation,     // 导航/目录
    Media,          // 媒体页面
    Documentation,  // 文档
    Unknown,        // 未知类型
}
```

**页面意图 (PageIntent)**：
```rust
pub enum PageIntent {
    Informational,   // 信息展示
    Transactional,   // 交易/操作
    Navigation,      // 导航/浏览
    Interactive,     // 交互/工具
    Entertainment,   // 娱乐/媒体
    Unknown,         // 未知意图
}
```

**可读性评分解释**：
- `0-30`: 非常难读（专业/技术内容）
- `30-50`: 难读（大学水平）
- `50-60`: 较难（高中水平）
- `60-70`: 标准（8-9年级）
- `70-80`: 较易（7年级）
- `80-90`: 容易（6年级）
- `90-100`: 非常容易（5年级）

**支持的语言**（部分列表）：
- `en`: English
- `zh`: Chinese
- `es`: Spanish
- `ja`: Japanese
- `fr`: French
- `de`: German
- `ru`: Russian
- `ar`: Arabic
- 等 60+ 种语言

**内部数据结构**：
```rust
SemanticAnalysis {
    content_type: ContentType,         // 内容类型
    intent: PageIntent,                // 页面意图
    language: String,                  // 语言代码
    language_confidence: f64,          // 置信度 [0.0-1.0]
    summary: String,                   // 摘要
    keywords: Vec<String>,             // 关键词列表
    readability: Option<f64>,          // 可读性评分
}
```

### 4. 跨模态洞察 (Cross-Modal Insights)

输出格式：
```
💡 Cross-Modal Insights
  • [InsightType] Description (confidence: XX%)
```

**洞察类型 (InsightType)**：

| 类型 | 说明 | 触发条件示例 |
|------|------|-------------|
| ContentStructureAlignment | 内容与结构对齐问题 | 简单内容但复杂DOM结构 |
| VisualSemanticConsistency | 视觉与语义一致性 | 内容多但视口利用率低 |
| AccessibilityIssue | 可访问性问题 | 低可读性 + 低对比度 |
| UserExperience | 用户体验观察 | 多模态组合发现的UX问题 |
| Performance | 性能相关 | DOM节点 > 2000 |
| Security | 安全相关 | 表单 + 不安全连接 |

**示例洞察**：

1. **性能洞察**：
   ```
   [Performance] Large DOM tree (3247 nodes) may impact rendering performance (confidence: 70%)
   ```
   - 条件：`dom_node_count > 2000`
   - 影响：渲染性能可能受影响
   - 建议：考虑优化 DOM 结构或使用虚拟滚动

2. **可访问性洞察**：
   ```
   [AccessibilityIssue] Low readability combined with poor contrast - accessibility concern (confidence: 80%)
   ```
   - 条件：`readability < 50.0 AND avg_contrast < 3.0`
   - 影响：视障用户难以阅读
   - 建议：提高对比度和文本可读性

3. **内容结构洞察**：
   ```
   [ContentStructureAlignment] Complex DOM structure for article content - may impact performance (confidence: 75%)
   ```
   - 条件：`dom_node_count > 1000 AND content_type == Article`
   - 影响：文章类内容不应有过于复杂的结构
   - 建议：简化 DOM 层级

4. **视觉语义洞察**：
   ```
   [VisualSemanticConsistency] Low viewport utilization for content-heavy page (confidence: 65%)
   ```
   - 条件：`viewport_utilization < 0.3 AND content_type == Article`
   - 影响：大量内容但显示空间利用不足
   - 建议：优化布局以更好利用屏幕空间

**内部数据结构**：
```rust
CrossModalInsight {
    insight_type: InsightType,       // 洞察类型
    description: String,             // 描述信息
    confidence: f64,                 // 置信度 [0.0-1.0]
    sources: Vec<PerceiverType>,     // 来源感知器
}
```

### 5. 整体置信度 (Overall Confidence)

输出格式：
```
Overall confidence: 87.5%
```

**计算方法**：
```rust
confidence = (
    structural_confidence * 0.4 +  // 结构权重 40%
    visual_confidence * 0.3 +      // 视觉权重 30%
    semantic_confidence * 0.3      // 语义权重 30%
) / weight_sum
```

**置信度解释**：
- `> 85%`: 高置信度 - 分析结果非常可靠
- `70-85%`: 中等置信度 - 分析结果可信
- `50-70%`: 低置信度 - 结果需要验证
- `< 50%`: 很低置信度 - 可能需要人工检查

影响因素：
- 结构分析：DOM 节点数 > 0 表示成功
- 视觉分析：截图捕获成功
- 语义分析：语言检测置信度

## 🧪 集成测试输出

运行集成测试时的输出示例：

```bash
$ SOULBROWSER_USE_REAL_CHROME=1 cargo test --test l2_perception_integration -- --nocapture

running 6 tests

test test_structural_perception ...
✓ Structural perception: 127 DOM nodes
ok

test test_visual_perception ...
✓ Visual perception:
  - Screenshot ID: screenshot_abc123def
  - Colors: 5
  - Contrast: 4.52
  - Viewport: 78.3%
ok

test test_semantic_perception ...
✓ Semantic perception:
  - Language: en (95.2%)
  - Content: Example
  - Intent: Informational
  - Keywords: ["example", "domain", "illustrative", "use"]
ok

test test_multimodal_perception ...
✓ Multi-modal perception:
  - Structural: 127 nodes
  - Visual: present
  - Semantic: present
  - Insights: 2
  - Confidence: 87.5%
ok

test test_cross_modal_insights ...
✓ Cross-modal insights test:
  - Generated 3 insights
  - [Performance] Large DOM tree (3247 nodes) may impact rendering performance (confidence: 70%)
  - [AccessibilityIssue] Low readability combined with poor contrast (confidence: 80%)
  - [ContentStructureAlignment] Complex DOM structure for article content (confidence: 75%)
ok

test test_perception_timeout ...
✓ Timeout handling verified
ok

test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 23.45s
```

## 📄 JSON 输出格式

使用 `--output results.json` 时的完整 JSON 结构：

```json
{
  "structural": {
    "snapshot_id": "snapshot_1234567890",
    "dom_node_count": 3247,
    "interactive_element_count": 145,
    "has_forms": true,
    "has_navigation": true
  },
  "visual": {
    "screenshot_id": "screenshot_abc123def",
    "dominant_colors": [
      [255, 255, 255],
      [0, 0, 0],
      [52, 101, 164],
      [230, 230, 230],
      [102, 102, 102]
    ],
    "avg_contrast": 4.52,
    "viewport_utilization": 0.783,
    "complexity": 0.67
  },
  "semantic": {
    "content_type": "Portal",
    "intent": "Navigation",
    "language": "en",
    "language_confidence": 0.952,
    "summary": "Wikipedia is a free online encyclopedia...",
    "keywords": [
      "wikipedia",
      "encyclopedia",
      "free",
      "knowledge",
      "articles"
    ],
    "readability": 62.3
  },
  "insights": [
    {
      "insight_type": "Performance",
      "description": "Large DOM tree (3247 nodes) may impact rendering performance",
      "confidence": 0.70,
      "sources": ["Structural"]
    },
    {
      "insight_type": "AccessibilityIssue",
      "description": "Low readability combined with poor contrast - accessibility concern",
      "confidence": 0.80,
      "sources": ["Visual", "Semantic"]
    }
  ],
  "confidence": 0.875
}
```

## 🔍 日志输出

使用 `RUST_LOG=info` 时的详细日志：

```
[2025-01-20T10:30:15Z INFO  soulbrowser] Starting multi-modal perception analysis
[2025-01-20T10:30:15Z INFO  cdp_adapter] Navigating to https://www.wikipedia.org
[2025-01-20T10:30:16Z INFO  cdp_adapter] DOM ready reached
[2025-01-20T10:30:16Z INFO  perceiver_structural] Capturing DOM snapshot
[2025-01-20T10:30:16Z INFO  perceiver_visual] Capturing screenshot
[2025-01-20T10:30:17Z INFO  perceiver_semantic] Extracting page text
[2025-01-20T10:30:17Z INFO  perceiver_semantic] Detected language: en (confidence: 0.952)
[2025-01-20T10:30:17Z INFO  perceiver_hub] Generating cross-modal insights
[2025-01-20T10:30:17Z INFO  perceiver_hub] Analysis complete (confidence: 0.875)
```

使用 `RUST_LOG=debug` 时的更详细输出：

```
[2025-01-20T10:30:15Z DEBUG perceiver_visual] Screenshot options: quality=80, format=PNG
[2025-01-20T10:30:16Z DEBUG perceiver_visual] Analyzing color palette...
[2025-01-20T10:30:16Z DEBUG perceiver_visual] Found 5 dominant colors
[2025-01-20T10:30:16Z DEBUG perceiver_visual] Computing contrast ratios...
[2025-01-20T10:30:17Z DEBUG perceiver_semantic] Tokenizing text (2347 words)
[2025-01-20T10:30:17Z DEBUG perceiver_semantic] Extracting keywords with TF-IDF
[2025-01-20T10:30:17Z DEBUG perceiver_semantic] Calculating readability score
[2025-01-20T10:30:17Z DEBUG perceiver_hub] Structural weight: 0.4, Visual weight: 0.3, Semantic weight: 0.3
```

## 📊 性能指标

典型的执行时间和资源使用：

| 操作 | 时间 | 内存 |
|------|------|------|
| 结构分析 | 100-300ms | ~10MB |
| 视觉分析 | 500-800ms | ~50MB |
| 语义分析 | 200-500ms | ~20MB |
| 多模态（并行） | 800-1500ms | ~80MB |
| 洞察生成 | 10-50ms | ~5MB |

## 🎯 使用建议

1. **首次使用**：建议用 `--all --insights` 获取完整信息
2. **性能敏感**：只启用需要的感知器（如 `--structural`）
3. **调试问题**：使用 `RUST_LOG=debug` 获取详细日志
4. **保存结果**：使用 `--output` 保存 JSON 供后续分析
5. **可视化验证**：使用 `--screenshot` 保存截图进行视觉确认

## 🔧 自定义输出

如果需要自定义输出格式，可以：

1. 读取 JSON 输出文件
2. 使用编程语言解析
3. 按需提取和格式化信息
4. 集成到自己的工具链中

示例（Python）：
```python
import json

with open('results.json') as f:
    data = json.load(f)

print(f"页面有 {data['structural']['dom_node_count']} 个节点")
print(f"检测语言: {data['semantic']['language']}")
print(f"置信度: {data['confidence'] * 100:.1f}%")
```
