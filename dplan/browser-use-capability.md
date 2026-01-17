# SoulBrowser: 实现 Browser-Use 风格 Agent Loop

## 问题分析

### 当前架构 (Plan-Execute)
```
用户请求 → LLM生成完整计划 → 顺序执行所有步骤 → 失败才重规划
```
- LLM 只在规划时参与一次
- 执行过程中不感知页面变化
- 无法动态调整策略

### 目标架构 (Observe-Think-Act Loop)
```
while !done && steps < max:
    state = observe()      # 获取当前页面状态
    action = llm.decide()  # LLM 根据实时状态决策
    result = execute()     # 执行 1-3 个动作
    if action.is_done: break
```
- 每步都调用 LLM
- 实时感知页面状态
- 动态适应变化

---

## 实现计划

### Phase 1: 核心数据结构

#### 1.1 添加 Done 动作
**文件**: `crates/agent-core/src/plan.rs`

```rust
pub enum AgentToolKind {
    // ... 现有变体 ...
    Done {
        success: bool,
        text: String,
    },
}
```

#### 1.2 创建 Agent Loop 类型
**新文件**: `crates/agent-core/src/agent_loop/types.rs`

```rust
/// 格式化后的浏览器状态 (供 LLM 消费)
pub struct BrowserStateSummary {
    pub url: String,
    pub title: Option<String>,
    pub element_tree: String,           // "[0]<button>Submit</button>"
    pub selector_map: HashMap<u32, ElementSelector>,
    pub screenshot_base64: Option<String>,
    pub scroll_position: ScrollPosition,
}

/// LLM 单步输出
pub struct AgentOutput {
    pub thinking: String,
    pub evaluation_previous_goal: Option<String>,
    pub memory: Option<String>,
    pub next_goal: String,
    pub actions: Vec<AgentAction>,
}

/// 单个动作
pub struct AgentAction {
    pub action_type: AgentActionType,
    pub element_index: Option<u32>,
    pub params: AgentActionParams,
}

pub enum AgentActionType {
    Navigate, Click, TypeText, Select, Scroll, Wait, Done,
}
```

#### 1.3 配置结构
**新文件**: `crates/agent-core/src/agent_loop/config.rs`

```rust
pub struct AgentLoopConfig {
    pub max_steps: u32,                    // 100
    pub max_actions_per_step: u32,         // 3
    pub max_consecutive_failures: u32,     // 3
    pub enable_vision: bool,               // true
    pub max_elements: u32,                 // 500
    pub action_timeout_ms: u64,            // 30000
    pub llm_timeout_ms: u64,               // 60000
}
```

---

### Phase 2: 浏览器状态格式化

#### 2.1 元素索引构建器
**新文件**: `crates/agent-core/src/agent_loop/element_tree.rs`

核心功能:
- 解析 DomAxSnapshot (来自 perceiver-structural)
- 过滤可交互元素 (button, input, link, select 等)
- 生成索引格式: `[0]<button>Click me</button>`
- 构建 selector_map: index → (css_selector, backend_node_id)

```rust
pub struct ElementTreeBuilder {
    max_elements: u32,
}

impl ElementTreeBuilder {
    pub fn build(&self, snapshot: &DomAxSnapshot) -> ElementTreeResult {
        // 遍历 DOM 节点
        // 过滤可交互元素
        // 分配连续索引
        // 返回格式化字符串 + selector_map
    }
}
```

#### 2.2 状态格式化器
**新文件**: `crates/agent-core/src/agent_loop/state_formatter.rs`

```rust
pub struct StateFormatter {
    element_builder: ElementTreeBuilder,
    enable_vision: bool,
}

impl StateFormatter {
    pub async fn format_state(
        &self,
        perception: &MultiModalPerception,
        url: &str,
        screenshot: Option<&str>,
    ) -> BrowserStateSummary;
}
```

---

### Phase 3: LLM 提供者扩展

#### 3.1 扩展 LlmProvider trait
**文件**: `crates/agent-core/src/llm_provider.rs`

```rust
#[async_trait]
pub trait LlmProvider: Send + Sync {
    // 现有方法...
    async fn plan(&self, request: &AgentRequest) -> Result<PlannerOutcome>;
    async fn replan(&self, ...) -> Result<PlannerOutcome>;

    // 新增: Agent Loop 决策方法
    async fn decide(
        &self,
        request: &AgentRequest,
        state: &BrowserStateSummary,
        history: &[AgentHistoryEntry],
    ) -> Result<AgentOutput, AgentError>;
}
```

#### 3.2 Agent Loop 提示词
**新文件**: `crates/soulbrowser-kernel/src/llm/agent_loop_prompt.rs`

System Prompt 核心内容:
- 角色定义: 浏览器自动化 Agent
- 输入格式: element_tree, history, task
- 输出格式: JSON with thinking/evaluation/memory/next_goal/actions
- 动作规则: 最多 3 个动作, done 必须单独
- 元素引用: 使用 [N] 索引

#### 3.3 Claude 实现
**文件**: `crates/soulbrowser-kernel/src/llm/anthropic.rs`

添加 `decide_impl` 方法:
- 构建消息 (支持 vision)
- 调用 Claude API
- 解析 AgentOutput JSON

---

### Phase 4: Agent Loop 控制器

**新文件**: `crates/agent-core/src/agent_loop/controller.rs`

```rust
pub struct AgentLoopController<L, P, A> {
    llm: Arc<L>,
    perception: Arc<P>,
    actions: Arc<A>,
    config: AgentLoopConfig,
}

impl AgentLoopController {
    /// 主循环
    pub async fn run(&self, request: &AgentRequest, route: &ExecRoute)
        -> Result<AgentLoopResult>
    {
        loop {
            // 检查终止条件
            if is_done || steps >= max_steps || failures >= max_failures {
                return result;
            }

            // 执行单步
            match self.execute_step(...).await {
                Ok(step_result) => {
                    history.push(step_result);
                    if step_result.is_done { break; }
                }
                Err(e) => {
                    consecutive_failures += 1;
                }
            }
        }
    }

    /// 单步执行: Observe → Think → Act
    async fn execute_step(&self, ...) -> Result<StepResult> {
        // 1. Observe
        let state = self.observe(route).await?;

        // 2. Think (LLM)
        let output = self.llm.decide(request, &state, &history).await?;

        // 3. Act
        for action in output.actions.iter().take(max_actions) {
            if action.is_done() {
                return Ok(StepResult::done(...));
            }
            self.execute_action(route, action, &state).await?;
        }

        Ok(StepResult::continue_loop(...))
    }

    /// 观察当前浏览器状态
    async fn observe(&self, route: &ExecRoute) -> Result<BrowserStateSummary> {
        let perception = self.perception.perceive(route, options).await?;
        self.state_formatter.format_state(&perception, ...).await
    }

    /// 执行单个动作
    async fn execute_action(&self, route: &ExecRoute, action: &AgentAction,
        state: &BrowserStateSummary) -> Result<ActionResult>
    {
        // 通过 element_index 查找 selector
        let selector = state.selector_map.get(&action.element_index);

        // 调用 ActionPrimitives
        match action.action_type {
            Navigate => self.actions.navigate(...),
            Click => self.actions.click(...),
            TypeText => self.actions.type_text(...),
            // ...
        }
    }
}
```

---

### Phase 5: 集成与测试

#### 5.1 执行模式选择
**文件**: `crates/agent-core/src/model.rs`

```rust
pub enum ExecutionMode {
    PlanExecute,  // 默认: 现有模式
    AgentLoop,    // 新增: browser-use 模式
}

pub struct AgentRequest {
    // ... 现有字段 ...
    pub execution_mode: ExecutionMode,
    pub agent_loop_config: Option<AgentLoopConfig>,
}
```

#### 5.2 模块导出
**文件**: `crates/agent-core/src/lib.rs`

```rust
pub mod agent_loop;
pub use agent_loop::{
    AgentLoopConfig, AgentLoopController, AgentLoopResult,
    AgentOutput, BrowserStateSummary,
};
```

---

## 文件清单

### 新建文件 (9个)

| 文件路径 | 用途 |
|---------|------|
| `crates/agent-core/src/agent_loop/mod.rs` | 模块入口 |
| `crates/agent-core/src/agent_loop/types.rs` | 数据结构 |
| `crates/agent-core/src/agent_loop/config.rs` | 配置 |
| `crates/agent-core/src/agent_loop/controller.rs` | 主循环逻辑 |
| `crates/agent-core/src/agent_loop/element_tree.rs` | 元素索引 |
| `crates/agent-core/src/agent_loop/state_formatter.rs` | 状态格式化 |
| `crates/soulbrowser-kernel/src/llm/agent_loop_prompt.rs` | 提示词模板 |
| `crates/agent-core/tests/agent_loop.rs` | 单元测试 |
| `crates/agent-core/tests/agent_loop_integration.rs` | 集成测试 |

### 修改文件 (7个)

| 文件路径 | 修改内容 |
|---------|---------|
| `crates/agent-core/src/plan.rs` | 添加 `AgentToolKind::Done` |
| `crates/agent-core/src/llm_provider.rs` | 添加 `decide()` 方法 |
| `crates/agent-core/src/model.rs` | 添加 `ExecutionMode` |
| `crates/agent-core/src/convert.rs` | 处理 Done 动作转换 |
| `crates/agent-core/src/lib.rs` | 导出 agent_loop 模块 |
| `crates/soulbrowser-kernel/src/llm/anthropic.rs` | 实现 `decide()` |
| `crates/agent-core/Cargo.toml` | 依赖更新 |

---

## 关键实现细节

### 元素索引格式 (与 browser-use 一致)
```
[0]<button class="submit">提交</button>
[1]<input type="text" placeholder="搜索...">
[2]<a href="/about">关于我们</a>
[3]<select name="category">
  [4]<option>选项1</option>
  [5]<option>选项2</option>
</select>
```

### LLM 输出格式 (JSON)
```json
{
  "thinking": "页面已加载，看到搜索框和提交按钮...",
  "evaluation_previous_goal": "导航成功，页面已显示",
  "memory": "目标网站: example.com, 任务: 搜索产品",
  "next_goal": "在搜索框输入关键词",
  "actions": [
    {"action": "type_text", "element_index": 1, "text": "iPhone 15", "submit": false},
    {"action": "click", "element_index": 0}
  ]
}
```

### 错误恢复策略
1. 单次动作失败 → 继续下一步 (让 LLM 看到错误并适应)
2. 连续 3 次失败 → 强制 done(success=false)
3. 达到 max_steps → 强制 done 并总结已完成内容

---

## 测试策略

### 单元测试
- `test_element_tree_building`: DOM 到索引树转换
- `test_agent_output_parsing`: LLM JSON 解析
- `test_loop_termination`: 各种终止条件

### 集成测试
- `test_simple_navigation`: 导航 → done
- `test_form_filling`: 填表单流程
- `test_error_recovery`: 错误恢复能力

### Mock 实现
```rust
pub struct MockAgentLoopLlm {
    responses: VecDeque<AgentOutput>,
}
```

---

## 验证方法

1. **单元测试**: `cargo test -p agent-core agent_loop`
2. **集成测试**: `cargo test -p agent-core --test agent_loop_integration`
3. **手动验证**:
   ```bash
   soulbrowser chat --prompt "搜索天气" --execution-mode agent-loop --execute
   ```
4. **对比测试**: 同任务分别用 plan-execute 和 agent-loop 模式执行，比较结果

---

## 实现顺序

1. **Phase 1** (数据结构): 2-3 小时
2. **Phase 2** (状态格式化): 3-4 小时
3. **Phase 3** (LLM 扩展): 2-3 小时
4. **Phase 4** (控制器): 4-5 小时
5. **Phase 5** (集成测试): 2-3 小时

**总计**: 约 15-18 小时开发工作
