# SoulBrowser Agent Loop 增强计划

## 问题总结

当前 Agent Loop 实现与 browser-use 相比存在以下关键差距：

1. **历史格式太简单** - 缺少 evaluation、memory 字段的实际使用
2. **System Prompt 不完整** - 缺少详细规则、示例和最佳实践
3. **元素树缺乏层级** - 扁平列表难以理解页面结构
4. **缺少动作结果反馈** - LLM 无法从错误中学习

### 额外修复 (2024-01)
5. **chromiumoxide 版本过旧** - 0.5.x 与新版 Chrome 不兼容，导致 "Failed to deserialize WS response" 错误
   - 已升级至 0.8.0，修复 CDP 通信问题

---

## 实现计划

### Phase 1: 增强 System Prompt ✅

**文件**: `crates/agent-core/src/agent_loop/prompt.rs`

**改进内容**:
- ✅ 添加截图作为 ground truth 的规则
- ✅ 添加多动作效率指南
- ✅ 添加任务完成规则
- ✅ 添加错误恢复策略
- ✅ 添加示例输出格式 (4个详细示例)

### Phase 2: 增强历史格式 ✅

**文件**: `crates/agent-core/src/agent_loop/prompt.rs` (format_user_message)

**改进内容**:
- ✅ 在历史中包含 evaluation_previous_goal
- ✅ 在历史中包含 memory
- ✅ 在历史中包含详细的动作结果
- ✅ 添加页面滚动上下文 (pages above/below)

### Phase 3: 增强元素树格式 ✅

**文件**: `crates/agent-core/src/agent_loop/element_tree.rs`

**改进内容**:
- ✅ 添加层级缩进支持
- ⬜ 跟踪新出现的元素（后续实现）

### Phase 4: 增强动作结果反馈 ✅

**文件**: `crates/soulbrowser-kernel/src/agent/agent_loop_executor.rs`

**改进内容**:
- ✅ 在 history 中记录更详细的动作结果
- ✅ 添加 evaluation 和 memory 字段

---

## 详细实现

### Phase 1: System Prompt 增强

```rust
// 新增内容要点：
// 1. 截图使用规则
// 2. 元素交互规则
// 3. 多动作效率指南
// 4. 任务完成规则
// 5. 错误恢复策略
// 6. 输出格式要求（包含 evaluation 和 memory）
```

### Phase 2: 历史格式增强

```rust
// format_user_message 中的历史部分改为：
// Step 1:
//   Evaluation: Success/Failed - 描述
//   Memory: 记住的关键信息
//   Actions: Click [5], TypeText [3]
//   Result: Success / Failed: error message
```

### Phase 3: 元素树层级

```rust
// format_tree 改为支持缩进：
// [0]<div class="form">
//   [1]<input type="text">
//   [2]<button>Submit</button>
// [3]<a href="/link">Link</a>
```

---

## 验证方法

1. 启动服务器：`cargo run -- serve --port 8808`
2. 发送 agent_loop 模式请求：
   ```bash
   curl -X POST http://localhost:8808/api/chat \
     -H "Content-Type: application/json" \
     -d '{"prompt": "打开百度搜索天气", "execution_mode": "agent_loop"}'
   ```
3. 观察日志中的 LLM 输入和输出
4. 验证任务完成情况

---

## 文件清单

| 文件 | 修改类型 | 优先级 |
|------|---------|--------|
| `crates/agent-core/src/agent_loop/prompt.rs` | 大幅增强 | P0 |
| `crates/agent-core/src/agent_loop/element_tree.rs` | 添加层级 | P1 |
| `crates/soulbrowser-kernel/src/agent/agent_loop_executor.rs` | 增强反馈 | P1 |
