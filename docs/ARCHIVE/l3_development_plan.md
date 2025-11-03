# L3 智能行动层（Intelligent Action）开发计划

**版本**: 1.0
**状态**: 规划阶段
**依赖**: L0 (CDP Adapter), L1 (Scheduler, State Center, Policy Center), L2 (All Perceivers)

## 📋 概述

L3 智能行动层是 SoulBrowser 的"执行肌肉"，负责将高层意图转化为可靠的浏览器操作。它包含四个核心模块：

1. **动作原语 (Primitives)** - 6个基础操作：navigate, click, type, select, scroll, wait
2. **定位与自愈 (Locator & Self-heal)** - CSS→ARIA/AX→Text 退避链，一次自动修复
3. **后验验收 (Post-conditions Gate)** - DOM/Network/URL/Title 多信号验证
4. **流程编排 (Flow Orchestration)** - 宏流程组合与条件分支

## 🎯 设计原则

### 核心理念
- **可组合**: 所有原语可自由组合
- **可中断**: 尊重取消令牌和 deadline
- **可追溯**: 完整的执行证据链
- **可解释**: 失败原因清晰明确
- **幂等性**: 相同输入产生相同结果

### 职责边界
- ✅ **做什么**: 执行动作、前置检查、保底等待、一次退避
- ❌ **不做什么**:
  - 不解析选择器（由 L2 负责）
  - 不决定策略（由 L5/L1 控制）
  - 不直接判定"完成"（由 Gate + L2 组合验证）

## 📦 模块架构

```
L3 Intelligent Action
├── action-primitives/          # 动作原语
│   ├── src/
│   │   ├── lib.rs             # 模块导出
│   │   ├── primitives.rs      # 原语 trait 定义
│   │   ├── navigate.rs        # navigate 实现
│   │   ├── click.rs           # click 实现
│   │   ├── type_text.rs       # type 实现
│   │   ├── select.rs          # select 实现
│   │   ├── scroll.rs          # scroll 实现
│   │   ├── wait.rs            # wait 实现
│   │   ├── models.rs          # 数据结构
│   │   └── errors.rs          # 错误类型
│   └── tests/                 # 单元测试
│
├── action-locator/            # 定位与自愈
│   ├── src/
│   │   ├── lib.rs
│   │   ├── locator.rs         # 定位器 trait
│   │   ├── fallback.rs        # 退避链逻辑
│   │   ├── heal.rs            # 自愈机制
│   │   ├── models.rs          # HealRequest/Outcome
│   │   └── errors.rs
│   └── tests/
│
├── action-gate/               # 后验验收
│   ├── src/
│   │   ├── lib.rs
│   │   ├── gate.rs            # Gate trait
│   │   ├── validators.rs      # 各类验证器
│   │   ├── expect_spec.rs     # 规则模型
│   │   ├── evidence.rs        # 证据收集
│   │   └── errors.rs
│   └── tests/
│
└── action-flow/               # 流程编排
    ├── src/
    │   ├── lib.rs
    │   ├── flow.rs            # Flow trait
    │   ├── sequence.rs        # 顺序执行
    │   ├── parallel.rs        # 并行执行
    │   ├── conditional.rs     # 条件分支
    │   └── errors.rs
    └── tests/
```

## 🔧 Phase 1: 动作原语 (Primitives)

### 1.1 核心数据结构

**执行上下文 (ExecCtx)**:
```rust
pub struct ExecCtx {
    pub route: ExecRoute,              // Session/Page/Frame 路由
    pub deadline: Instant,             // L1 下发的截止时间
    pub cancel_token: CancellationToken, // 取消令牌
    pub policy_view: PolicyView,       // Policy 子视图
    pub action_id: String,             // 用于时间线对齐
}
```

**动作报告 (ActionReport)**:
```rust
pub struct ActionReport {
    pub ok: bool,
    pub started_at: Instant,
    pub finished_at: Instant,
    pub latency_ms: u64,
    pub precheck: Option<PrecheckResult>, // 元素类原语
    pub post_signals: PostSignals,     // 轻量快照
    pub self_heal: Option<SelfHealInfo>,
    pub error: Option<ActionError>,
}

pub struct PostSignals {
    pub url_changed: bool,
    pub title_changed: bool,
    pub dom_diff_count: usize,         // L2 局部结构变化
    pub network_2xx_count: usize,      // 网络成功请求
    pub network_quiet_ms: u64,         // 网络安静时长
}
```

### 1.2 系统保底等待

两档内建等待：

1. **domready**: 等待 `domContentLoaded` 事件
2. **idle**: domready + 轻量 network-idle（inflight == 0 && 静默 ≥ 1000ms）

```rust
pub enum WaitTier {
    None,      // 不等待（策略可禁止）
    DomReady,  // 等待 DOM 就绪
    Idle,      // 等待网络安静
}
```

### 1.3 六个原语实现

#### navigate(url, wait_tier=idle)
```rust
pub async fn navigate(
    ctx: &ExecCtx,
    url: &str,
    wait_tier: WaitTier,
) -> Result<ActionReport>;
```

- **前置**: Permissions 放行（首次导航/跨域）
- **执行**: CDP Page.navigate → wait(domready) → [wait(idle)]
- **后置**: 最终 URL、标题、重定向信息、Network 摘要

#### click(anchor, wait_tier=domready)
```rust
pub async fn click(
    ctx: &ExecCtx,
    anchor: &AnchorDescriptor,
    wait_tier: WaitTier,
) -> Result<ActionReport>;
```

- **前置**: is_clickable 检查、scrollIntoView、focus
- **执行**: 注入鼠标事件（down→up/click）
- **退避**: 一次备用锚点（AX→文本→CSS）
- **后置**: 焦点变化、结构差分、Network 计数

#### type_text(anchor, text, submit, wait_tier=domready)
```rust
pub async fn type_text(
    ctx: &ExecCtx,
    anchor: &AnchorDescriptor,
    text: &str,
    submit: bool,
    wait_tier: WaitTier,
) -> Result<ActionReport>;
```

- **前置**: is_enabled 检查、focus
- **执行**: 键入文本（可选人类节奏），submit=true 发送 Enter
- **安全**: 密码字段不落盘、不可回显
- **后置**: 目标 value 变更摘要、光标位置

#### select(anchor, by, item, wait_tier=domready)
```rust
pub enum SelectBy {
    Value(String),
    Label(String),
    Index(usize),
}

pub async fn select(
    ctx: &ExecCtx,
    anchor: &AnchorDescriptor,
    by: SelectBy,
    wait_tier: WaitTier,
) -> Result<ActionReport>;
```

- **前置**: is_enabled 检查、滚动至可见
- **执行**: 变更 selected 项（原生事件 + 变更触发）
- **后置**: 选中项列表、结构差分

#### scroll(to, behavior)
```rust
pub enum ScrollTarget {
    Anchor(AnchorDescriptor),
    Y(f64),
    Delta(f64),
    ElementCenter(AnchorDescriptor),
}

pub async fn scroll(
    ctx: &ExecCtx,
    to: ScrollTarget,
    behavior: ScrollBehavior,
) -> Result<ActionReport>;
```

- **执行**: 滚动容器或页面（smooth/instant）
- **后置**: 视口范围变化、目标是否进入可视

#### wait(kind, timeout)
```rust
pub enum WaitKind {
    Evaluate(String),              // JS 表达式
    DomReady,                      // domContentLoaded
    Idle,                          // network-idle
    SelectorVisible(String),       // 选择器可见
    NetworkQuiet(u64),            // 网络安静 N ms
    Event(String),                // 自定义事件
}

pub async fn wait(
    ctx: &ExecCtx,
    kind: WaitKind,
    timeout: Duration,
) -> Result<ActionReport>;
```

### 1.4 错误模型

```rust
#[derive(Debug, Error)]
pub enum ActionError {
    #[error("Navigation timeout")]
    NavTimeout,
    #[error("Wait timeout")]
    WaitTimeout,
    #[error("Operation interrupted")]
    Interrupted,
    #[error("Element not clickable: {0}")]
    NotClickable(String),
    #[error("Element not enabled: {0}")]
    NotEnabled(String),
    #[error("Option not found: {0}")]
    OptionNotFound(String),
    #[error("Anchor not found: {0}")]
    AnchorNotFound(String),
    #[error("Scroll target invalid: {0}")]
    ScrollTargetInvalid(String),
    #[error("Stale route: {0}")]
    StaleRoute(String),
    #[error("CDP IO error: {0}")]
    CdpIo(String),
    #[error("Policy denied: {0}")]
    PolicyDenied(String),
    #[error("Internal error: {0}")]
    Internal(String),
}
```

每个错误附带：
- **hint**: 下一步建议
- **retryable**: 是否可退避标记

### 1.5 State Center 集成

写入时间线事件：
```rust
pub enum ActionEvent {
    Started { action_id, tool_name, route, wait_tier },
    Finished { action_id, latency_ms, ok, error },
    Precheck { action_id, visible, clickable, enabled },
    SelfHeal { action_id, attempted, reason, used_anchor },
    PostSignals { action_id, signals },
}
```

脱敏原则：
- 不记录明文输入（仅长度/摘要 Hash）
- 不记录像素
- URL 查询值打码

## 🔧 Phase 2: 定位与自愈 (Locator & Self-heal)

### 2.1 退避链策略

固定三层退避链：**CSS → ARIA/AX → Text**

```rust
pub enum LocatorStrategy {
    Css(String),                    // CSS 选择器
    AriaAx { role: String, name: String }, // ARIA role + name
    Text { content: String, exact: bool }, // 文本内容
}

pub struct FallbackPlan {
    pub primary: AnchorDescriptor,
    pub fallbacks: Vec<Candidate>,  // 按优先级排序
}

pub struct Candidate {
    pub from: LocatorStrategy,
    pub anchor: AnchorDescriptor,
    pub score: f32,                 // L2 评分
    pub precheck: PrecheckResult,   // visible/clickable/enabled
}
```

### 2.2 自愈触发条件

```rust
pub enum HealReason {
    NotClickable,        // is_clickable = false
    NotEnabled,          // is_enabled = false
    NoEffect,            // 点击/输入后无显著变化
    Ambiguous,           // 多个匹配，需要消歧
}
```

触发时机：
1. **前置失败**: is_visible/is_clickable/is_enabled = false
2. **注入疑似无效**: 保底等待后，DOM diff ≈ 0 且 Network 信号无变化
3. **Gate 未达成**: Post-Condition 未通过且标记"定位可疑"

### 2.3 自愈流程

```rust
pub async fn try_once(
    ctx: &ExecCtx,
    primary: &AnchorDescriptor,
    reason: HealReason,
) -> Result<HealOutcome>;

pub enum HealOutcome {
    Healed { used_anchor: AnchorDescriptor },
    Skipped { reason: String },
    Exhausted { candidates: Vec<Candidate> },
    Aborted,
}
```

流程：
1. **构建退避计划**: 基于 primary.strategy 生成候选链
2. **过滤候选**: 移除明显不可用项（invisible/disabled）
3. **择优选择**: 按评分和 precheck 结果选择最佳候选
4. **返回结果**: 附带完整证据链

限制：
- **一次退避**: 每个原语调用最多一次
- **时间预算**: 必须在剩余 deadline 内完成
- **候选上限**: Top-K（默认 K=3）

### 2.4 失败证据

```rust
pub struct FailureEvidence {
    pub attempted_strategies: Vec<LocatorStrategy>,
    pub candidates_tried: Vec<Candidate>,
    pub dom_snapshot_digest: String,    // 结构快照摘要
    pub network_state: NetworkState,    // 网络状态
    pub visual_hint: Option<String>,    // 可选视觉提示
    pub explain: String,                // 失败原因说明
}
```

## 🔧 Phase 3: 后验验收 (Post-conditions Gate)

### 3.1 规则模型 (ExpectSpec)

```rust
pub struct ExpectSpec {
    pub timeout_ms: u64,
    pub all: Vec<Condition>,        // 全部满足
    pub any: Vec<Condition>,        // 任一满足
    pub deny: Vec<Condition>,       // 否决条件
    pub locator_hint: LocatorHint,  // 定位可疑判据
}

pub enum Condition {
    Dom(DomCondition),
    Net(NetCondition),
    Url(UrlCondition),
    Title(TitleCondition),
    Runtime(RuntimeCondition),
    Vis(VisCondition),
    Sem(SemCondition),
}
```

#### DOM 条件
```rust
pub struct DomCondition {
    pub diff_near: DiffScope,       // anchor | region
    pub min_changes: usize,
    pub state_is: Option<ElementState>,
}

pub enum DiffScope {
    Anchor,                         // 锚点附近
    Region { selector: String },    // 指定区域
    Global,                         // 全局
}
```

#### Network 条件
```rust
pub struct NetCondition {
    pub any_2xx_on: Vec<String>,    // URL 模式
    pub forbid_4xx5xx: bool,
    pub quiet_ms: u64,
}
```

#### URL/Title 条件
```rust
pub struct UrlCondition {
    pub changes: bool,
    pub contains: Option<String>,
    pub equals: Option<String>,
}

pub struct TitleCondition {
    pub changes: bool,
    pub contains: Option<String>,
}
```

### 3.2 Gate 执行

```rust
pub async fn verify(
    ctx: &ExecCtx,
    action_id: &str,
    spec: &ExpectSpec,
) -> Result<GateResult>;

pub struct GateResult {
    pub pass: bool,
    pub since: Instant,
    pub until: Instant,
    pub reasons: Vec<String>,
    pub evidence: Evidence,
    pub suggest_heal: bool,         // 是否建议自愈
}
```

验证流程：
1. **采集证据**: 从 L2 Perceivers 和 L0 Network Tap 收集信号
2. **评估条件**: 检查 all/any/deny 规则
3. **生成结论**: pass/fail + 详细原因
4. **触发退避**: 如果标记"定位可疑"且允许 heal

### 3.3 证据包

```rust
pub struct Evidence {
    pub dom: DomEvidence,
    pub net: NetEvidence,
    pub url: UrlEvidence,
    pub title: TitleEvidence,
    pub runtime: RuntimeEvidence,
    pub vis: Option<VisEvidence>,
    pub sem: Option<SemEvidence>,
}

pub struct DomEvidence {
    pub diff_digest: String,
    pub changes_count: usize,
    pub anchor_state: Option<ElementState>,
}

pub struct NetEvidence {
    pub summary: NetworkSummary,    // 2xx/4xx/5xx 计数
    pub quiet_at_ms: u64,
    pub matched_txn: Vec<String>,   // 匹配的事务
}
```

## 🔧 Phase 4: 流程编排 (Flow Orchestration)

### 4.1 流程类型

```rust
pub enum Flow {
    Sequence(Vec<Step>),            // 顺序执行
    Parallel(Vec<Step>),            // 并行执行
    Conditional(ConditionalFlow),   // 条件分支
    Loop(LoopFlow),                 // 循环
}

pub struct Step {
    pub action: PrimitiveAction,
    pub gate: Option<ExpectSpec>,
    pub on_fail: FailureStrategy,
}

pub enum FailureStrategy {
    Abort,                          // 中止流程
    Continue,                       // 继续下一步
    Retry(RetryPolicy),            // 重试
    Fallback(Box<Flow>),           // 回退方案
}
```

### 4.2 条件流程

```rust
pub struct ConditionalFlow {
    pub condition: Condition,
    pub then_flow: Box<Flow>,
    pub else_flow: Option<Box<Flow>>,
}

// 支持的条件类型
pub enum FlowCondition {
    ElementVisible(AnchorDescriptor),
    UrlContains(String),
    TitleEquals(String),
    NetworkSuccess(String),
    Custom(String),                 // JS 表达式
}
```

### 4.3 循环流程

```rust
pub struct LoopFlow {
    pub count: Option<usize>,       // 固定次数
    pub while_cond: Option<Condition>, // 条件循环
    pub body: Box<Flow>,
    pub max_iterations: usize,      // 安全上限
}
```

## 📅 开发时间表

### Week 1-2: Phase 1 - 动作原语
- [ ] Day 1-2: 核心数据结构和 trait 定义
- [ ] Day 3-4: navigate 和 wait 实现
- [ ] Day 5-6: click 和 type_text 实现
- [ ] Day 7-8: select 和 scroll 实现
- [ ] Day 9-10: 单元测试和集成测试

### Week 3: Phase 2 - 定位与自愈
- [ ] Day 1-2: 退避链逻辑和候选生成
- [ ] Day 3-4: 自愈机制实现
- [ ] Day 5: 失败证据收集
- [ ] Day 6-7: 测试和文档

### Week 4: Phase 3 - 后验验收
- [ ] Day 1-2: 规则模型和条件解析
- [ ] Day 3-4: 证据收集和验证逻辑
- [ ] Day 5-6: Gate 执行和触发机制
- [ ] Day 7: 测试和文档

### Week 5: Phase 4 - 流程编排
- [ ] Day 1-2: 基础流程类型（Sequence, Parallel）
- [ ] Day 3-4: 条件和循环流程
- [ ] Day 5-6: 失败策略和回退
- [ ] Day 7: 测试和文档

### Week 6: 集成与优化
- [ ] Day 1-2: 端到端集成测试
- [ ] Day 3-4: 性能优化和稳定性测试
- [ ] Day 5-6: CLI 命令集成
- [ ] Day 7: 文档完善和示例

## 🧪 测试策略

### 单元测试
- 每个原语的独立测试
- 模拟 CDP 响应测试各种场景
- 错误路径覆盖
- 退避链逻辑测试

### 集成测试
- 真实浏览器测试（需要 SOULBROWSER_USE_REAL_CHROME=1）
- 与 L2 Perceivers 集成测试
- Gate 验证测试
- 流程编排测试

### 性能测试
- 原语执行延迟 < 100ms（不含网络等待）
- 自愈决策 < 50ms
- Gate 验证 < 200ms

### 压力测试
- 并发执行稳定性
- 取消和超时处理
- 资源泄漏检查

## 📝 示例用法

### 基础原语使用

```rust
use action_primitives::*;

// Navigate
let report = navigate(&ctx, "https://example.com", WaitTier::Idle).await?;

// Click with auto-heal
let anchor = AnchorDescriptor::css("#submit");
let report = click(&ctx, &anchor, WaitTier::DomReady).await?;

// Type with submit
let input = AnchorDescriptor::aria("textbox", "Search");
let report = type_text(&ctx, &input, "SoulBrowser", true, WaitTier::DomReady).await?;
```

### 带 Gate 验证

```rust
use action_gate::*;

// 点击后验证
let spec = ExpectSpec {
    timeout_ms: 2000,
    all: vec![
        Condition::Dom(DomCondition {
            diff_near: DiffScope::Anchor,
            min_changes: 1,
            state_is: None,
        }),
        Condition::Net(NetCondition {
            any_2xx_on: vec!["/api/submit".to_string()],
            forbid_4xx5xx: true,
            quiet_ms: 800,
        }),
    ],
    any: vec![],
    deny: vec![],
    locator_hint: LocatorHint::SuspiciousIfNoDomEffect,
};

let report = click(&ctx, &anchor, WaitTier::DomReady).await?;
let result = verify(&ctx, &report.action_id, &spec).await?;

if !result.pass && result.suggest_heal {
    // 尝试自愈
    let outcome = try_once(&ctx, &anchor, HealReason::NoEffect).await?;
    if let HealOutcome::Healed { used_anchor } = outcome {
        let report = click(&ctx, &used_anchor, WaitTier::DomReady).await?;
    }
}
```

### 流程编排

```rust
use action_flow::*;

let flow = Flow::Sequence(vec![
    Step {
        action: PrimitiveAction::Navigate { url: "https://example.com".into() },
        gate: Some(url_contains("example.com")),
        on_fail: FailureStrategy::Abort,
    },
    Step {
        action: PrimitiveAction::Click { anchor: search_button },
        gate: Some(dom_changes_near_anchor(1)),
        on_fail: FailureStrategy::Retry(RetryPolicy::fixed(3)),
    },
    Step {
        action: PrimitiveAction::TypeText {
            anchor: search_input,
            text: "query".into(),
            submit: true,
        },
        gate: Some(network_2xx_on("/search")),
        on_fail: FailureStrategy::Abort,
    },
]);

let result = execute_flow(&ctx, &flow).await?;
```

## 🔗 依赖关系

### 下游依赖
- **L0 CDP Adapter**: 所有浏览器操作
- **L0 Permissions Broker**: 权限放行
- **L0 Network Tap**: 网络信号
- **L2 Structural Perceiver**: 锚点解析、元素检查
- **L2 Visual Perceiver**: 视觉证据（可选）
- **L2 Semantic Perceiver**: 语义证据（可选）

### 上游消费
- **L5 Tools Layer**: 组合原语实现高级工具
- **L1 Scheduler**: 调度和取消控制
- **L1 State Center**: 时间线记录

### 同层协同
- Primitives ↔ Locator: 自愈触发
- Primitives ↔ Gate: 后验验证
- Gate → Locator: 定位可疑触发

## 📚 参考文档

- [L3 动作原语逻辑规约](/mnt/d/github/SoulBrowserClaude/L3 智能行动（Intelligent Action）/01-动作原语（Primitives）/逻辑规约.md)
- [L3 定位与自愈逻辑规约](/mnt/d/github/SoulBrowserClaude/L3 智能行动（Intelligent Action）/02-定位与自愈（Locator & Self-heal）/逻辑规约.md)
- [L3 后验验收逻辑规约](/mnt/d/github/SoulBrowserClaude/L3 智能行动（Intelligent Action）/03-后验验收（Post-conditions Gate）/逻辑规约.md)
- [L3 流程编排逻辑规约](/mnt/d/github/SoulBrowserClaude/L3 智能行动（Intelligent Action）/04-流程编排（Macro Flow)/逻辑规约.md)

## 🎯 成功标准

### 功能完整性
- [ ] 6个原语全部实现并测试通过
- [ ] 退避链能正确处理 CSS→ARIA/AX→Text
- [ ] Gate 能正确验证所有条件类型
- [ ] 流程编排支持所有基础模式

### 性能指标
- [ ] 原语执行延迟 < 100ms（不含等待）
- [ ] 自愈决策 < 50ms
- [ ] Gate 验证 < 200ms
- [ ] 支持 10+ 并发执行

### 可靠性
- [ ] 100% 测试覆盖率（核心路径）
- [ ] 所有错误路径都有清晰的 hint
- [ ] 取消和超时 100% 可靠
- [ ] 无资源泄漏

### 可用性
- [ ] API 简洁易用
- [ ] 错误信息清晰
- [ ] 文档完整
- [ ] 示例代码丰富

---

**下一步**: 开始 Phase 1 - 动作原语实现
