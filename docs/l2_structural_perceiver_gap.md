# L2 · Structural Perceiver 实现差距记录

> 2025-02-XX · 记录当前 `crates/perceiver-structural` 与《技术实现文档》之间的差异，作为 P0 基线对齐的输入。

## 1. 模块结构

| 规约要求 | 现状 | 差距 |
|----------|------|------|
| `resolver/{generate,rank,reason}` | 已有 `generate.rs`/`rank.rs`，新增 `reason.rs` 占位 | 需将可解释组件从占位扩展为完整实现 |
| `judges.rs`（多信号判定） | 简单几何/属性提示 | 未整合 AX 状态、Style 判定、Policy 阈值 |
| `differ.rs`（去抖/聚焦） | 集合差分（节点/角色/文本） | 无 `SnapLevel`、`FocusSpec`、`debounce` 策略 |
| `redact.rs` | 占位实现 | 需接入策略化脱敏/裁剪 |
| `metrics.rs` | 占位计数器 | 需落地 metrics crate + 延迟统计 |
| `events.rs` | tracing + StateCenter 基础字段 | 缺少完整 Score/Diff payload 与回放数据 |
| `policy.rs` | 已含 `ResolveOptions/ScoreWeights/Judge/Diff` | 需接入 Policy Center 热加载，并补齐权重配置 |
| `sampler.rs` | 已支持 Light/Full 采样并加入重试 | 仍缺 SnapshotId/时间同步 |

## 2. 公共 API

| 规约 | 现状 | 差距 |
|------|------|------|
| `SelectorOrHint`（多模输入） | 已新增占位枚举 | Attr/AX/Fuzzy 仍待实现；Combo 仍走首选项 |
| `ResolveOpt`（候选数/去抖） | 新结构已包含 fuzziness/debounce | 尚未在 resolver 中完全生效 |
| `Scope` / `SnapLevel` | 已引入类型 | `snapshot_dom_ax_ext` 仍为占位实现 |
| `DomAxSnapshot`（id/ts/page/frame） | 已携带 id/时间戳/级别 | Light 级别与 partial AX 尚未实现 
| `diff_dom_ax(base_id,new_id)` | `diff_with_policy` 支持 `max_changes` | FocusSpec、debounce 仍缺 |
| `advice_for_interaction` | Trait 有默认实现 | 未提供真实建议逻辑 |

## 3. 候选生成与评分

- 现状：新增 `ScoreBreakdown` + 权重占位，但候选/权重仍为启发式；理由生成基于占位。
- 规约：AX/Attr/Text/Fuzzy/Combo 候选；Policy 权重驱动；Score → Reason 输出（可解释）。

## 4. 判定/差分

- `judges.rs`：未读取 Style/AX state，未结合 Policy 阈值，facts 仅 geometry。
- `differ.rs`：引入 `DiffPolicy::max_changes` 与 debounce；仍缺 FocusSpec 与更细粒度聚焦/去抖实现。

## 5. Policy / 缓存

- 现状：缓存 TTL 固定 250 ms/1 s；无 Policy hook。
- 规约：`PerceiverPolicy` 包含权重、TTL、去抖窗口；需从 Policy Center 读取。

## 6. Telemetry / State Center / CLI

- 现状：仅 tracing 日志；State Center 只存 resolve/judge/snapshot/diff 简易信息。
- 规约：写入 Score/Reason/Diff 细节，可回放；CLI 要有全面展示。

## 7. CDP 端口

- 现状：`AdapterPort` 包含 `query()`、`dom_snapshot()`、`ax_snapshot()`。
- 规约：需要 `query_attr`、`get_geometry`、`get_styles` 等 API，支持 partial AX。

---

> **下一步（P0）：**
>
> 1. 按规范建立 crate 结构与占位模块。
> 2. 扩展模型/API（保留旧接口兼容）。
> 3. 搭建差异化测试骨架，为后续阶段迭代做准备。
