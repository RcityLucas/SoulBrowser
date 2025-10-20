# L2 · Structural Perceiver 实施计划

> 目标：落地文档《L2 分层感知 / Structural Perceiver（DOMSnapshot + AX）》所描述的能力，使当前 `crates/perceiver-structural` 实现与规约 100% 对齐，并与 L0/L1/L5 模块无缝协作。

## 总览

| 阶段 | 时间估算 | 关键交付 | 完成信号 |
|------|----------|----------|----------|
| **P0 基线对齐** | ~1.5 周 | crate 结构重构、API 对齐（SelectorOrHint/ResolveOpt/SnapLevel 等）、兼容层与基础测试 | `cargo test -p perceiver-structural` + 旧 CLI 行为无回归 |
| **P1 候选生成/推理增强** | ~2.5 周 | 多模候选、ScoreBreakdown、Policy 权重、短期缓存/去抖、Telemetry（metrics） | 新 API 在 feature flag 下通过集成测试；metrics 输出在调试日志中可见 |
| **P2 判定/差分与脱敏** | ~2 周 | DOM/AX 判定深度化、Diff 去抖/聚焦、文本脱敏、InteractionAdvice | `judge`/`diff` 测试覆盖率 >80%，Advice 输出包含结构化建议 |
| **P3 端口与状态中心集成** | ~1.5 周 | 完整 CDP 端口、SnapLevel 支持、State Center 事件拓展、CLI 感知历史增强 | `soulbrowser perceiver` 输出 Score/Reason/Diff 摘要；State Center 可回放 |
| **P4 打磨与验证** | ~2 周 | Policy 热加载、端到端回放测试、性能基准、文档/迁移指南 | QA 签收；文档更新；CI 增加 perceiver 专项流程 |

## P0 · 基线对齐（~1.5 周）

1. **差距审计**
   - 对比现有实现与规约，输出差距清单（接口、模块、策略、Telemetry）。
   - 提交 `docs/l2_structural_perceiver_gap.md` 记录现状。

2. **crate 重构**
   - 创建规范目录结构：`reason.rs`、`redact.rs`、`metrics.rs`、`events.rs` 等文件。
   - 迁移现有逻辑到新模块，保留旧接口 (feature flag `legacy_api`)，方便渐进迁移。

3. **API 扩展**
   - 在 `model.rs` 引入 `SelectorOrHint`、`ResolveOpt`、`Scope`、`SnapLevel`、`SnapshotId`。
   - 升级 `AnchorResolution`、`JudgeReport`（结构化 facts），`DomAxSnapshot`（包含 id/ts）。
   - 调整 `api.rs` Trait，新接口与旧 CLI 兼容（过渡适配器）。

4. **测试**
   - 补齐 Parse/Resolve/Judge/Diff 的单元测试骨架，保证重构后功能无回归。

## P1 · 候选生成与推理增强（~2.5 周）

1. **多模候选** (`resolver::generate`)
   - 支持 AX/Attr/Text/Fuzzy/Combo；添加模糊匹配策略（配置驱动）。
   - `generate` 输出 `ScoreBreakdown` 初值。

2. **Scoring 与 Reason** (`resolver::rank`, `reason.rs`)
   - 根据 Policy 权重（AX role、可见性、文本匹配、后端节点等）计算最终分数。
   - ✔️ 初步实现 ScoreBreakdown + 可解释字符串（仍为占位权重，待 Policy 接入）。

3. **Policy 与缓存**
   - 创建 `policy.rs`：从 Policy Center 读取 `ResolvePolicy`、`ScoreWeights`、`CacheTtl`。
   - `cache.rs` 引入 anchor/snapshot 多级 TTL+去抖；缓存命中写指标。

4. **指标**
   - `metrics.rs`：`resolve_total`、`judge_latency`、`cache_hit_ratio` 等；暂输出到 tracing，后续挂接 metrics crate。

## P2 · 判定/差分与脱敏（~2 周）

1. **可见/可点/启用** (`judges.rs`)
   - DOM Styles、AX 状态、几何交集判定；支持 Policy 阈值（可配置）。
   - 报告中记录事实（style flags、AX 属性、几何面积等）。

2. **差分** (`differ.rs`)
   - 实现 `SnapLevel::Light/Full`、`FocusSpec`、`DiffPolicy`（去抖窗口、最大变更数）；采样需带重试并兼容无法获取 AX 的场景。
   - 输出结构化变更类型（节点新增/属性变化/文本 diff/AX role 变化等）和摘要。

3. **脱敏** (`redact.rs`)
   - 文本截断/正则白名单/敏感字段掩码；在所有对外数据流前调用。

4. **互动建议** (`api.rs` / `structural.rs`)
   - 根据 Judge、Score、Diff 情况生成 `InteractionAdvice`（如“建议使用 aria-name”、“需等待可见”等）。

## P3 · 端口与状态中心集成（~1.5 周）

1. **CDP 端口** (`ports.rs`)
   - 扩展接口 `ax_snapshot(partial)`、`query_attr`、`get_styles`、`get_geometry`。
   - 完善错误映射与重试策略；保持与 cdp-adapter 对齐。

2. **采样器** (`sampler.rs`)
   - 根据 SnapLevel 调度采样；返回 `SampledSnapshot` + 索引。
   - 生成 `SnapshotId` 以便 diff 与缓存。

3. **State Center & CLI**
   - `events.rs`：记录 ScoreBreakdown、Judge facts、Diff 摘要、Advice；确保可回放。
   - 升级 `soulbrowser perceiver` 命令展示理由、变更摘要；导出 JSON。

## P4 · 打磨与验证（~2 周）

1. **Policy 热加载**
   - 监听 Policy Center 变更，动态更新权重/阈值/TTL；提供 CLI override。

2. **端到端回放测试**
   - 使用真实 DOMSnapshot/AX fixture；构建 replay harness 验证 resolve→judge→diff→advice 全链路。
   - 引入不稳定 DOM（节点变化）场景，验证去抖与 fallback。

3. **性能与稳定性**
   - 缓存命中率 & 延迟统计；并发压测；Tracing 采样配置。
   - 文本脱敏审计，确保不泄露敏感数据。

4. **文档与迁移**
   - 更新技术实现文档、功能规约、CLI 帮助、迁移指引。
   - 将旧 API 行为文档化，提供跨版本指南。

---

## 验收标准

- 所有 CLI/文档与实现保持一致，`cargo test -p perceiver-structural` / `-p soulbrowser-cli` 全绿。
- Telemetry（State Center、metrics、Tracing）输出包含 Score/Reason/Diff 等完整信息。
- 开发计划执行后，文档《Structural Perceiver 技术实现》即可被视为“实现完成”。
