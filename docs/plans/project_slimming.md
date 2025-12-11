# 项目精简计划

> 目标：识别并移除 SoulBrowser 中的冗余结构、过时实现和重复文档，降低心智负担，让新成员在最短时间内掌握架构。以下计划按“信息→代码→依赖→流程”四个层面分组，每项都给出处置建议和预期成果。

## 1. 文档与信息结构
- **索引统一**：对 `docs/*` 做一次审计，确保所有仍需保留的内容都在 `docs/README.md` 挂载；将已被新版文档覆盖的条目迁移到 `docs/ARCHIVE/` 并注明弃用原因。✅ `docs/README.md` 已补充 Guides/Monitoring/Examples 等目录入口，后续只需标记废弃条目并迁移。 
- ✅ **冗余指南合并**：`guides/` 目录中存在重复说明（如 BACKEND_USAGE、START_SERVER、TROUBLESHOOTING），提炼成“带起 + 故障排查”两篇长文，其余以链接形式引用，减少“同主题多版本”。`BACKEND_USAGE.md` 现缩减为 quick reference，并指向 `TROUBLESHOOTING.md`（主入口）与 `docs/ARCHIVE/BACKEND_USAGE_LEGACY.md`，旧的 `START_SERVER.md`、`Perceive_API_浏览器问题解决.md` 仍归档并标注替代文档。 
- ✅ **计划文档去重**：新计划（Serve/API、精简计划等）使用统一模板：背景、范围、行动项、完成定义，而不是散落在 README/Issue 中；废弃或完成的计划移至 `docs/plans/ARCHIVE/`。历史的 L8 Agent 计划（CDP、Perception-first、User Need、Stage1 Progress）已全部迁入该目录并在 `docs/agent/README.md` 提示，仅保留现行的 Serve/API 与 Slimming 计划。
- ✅ **示例与脚本注释**：`examples/README.md` 仅列出仍维护的 DSL/SDK 示范，其余 Rust demo + 旧脚本集中记录在 `docs/examples/legacy_examples.md`，需要时再查阅，默认索引不再铺开这些历史资产。 

## 2. 代码结构精简
- **模块分层整理**：`
  - `src/main.rs` 长达数千行，按子域（CLI command handlers、Serve 路由、任务查询、记忆中心等）拆分成 `src/cli/*` 与 `src/server/*`，导出清晰接口。 
  - 将 `ServeState` / rate limiter / perception handler 等独立到 `src/server/serve.rs`、`src/server/rate_limit.rs`，降低单文件复杂度。 
- ✅ 首批拆分：`cmd_serve`、`ServeArgs` 迁移到 `src/cli/serve.rs`，`src/main.rs` 仅保留命令分派逻辑，其余 Serve 启动细节集中至模块，后续可继续拆分任务/记忆等子域。 
- ✅ Gateway CLI 同步拆分：`GatewayArgs`/`cmd_gateway`/策略加载已迁移到 `src/cli/gateway.rs`，主文件只保留运行时 handler，方便继续瘦身 Task/Gateway 逻辑。
- ✅ CLI 子命令持续瘦身：内存相关 Args/handlers (`cmd_memory`) 已迁入 `src/cli/memory.rs`，共用的模板/标签工具通过模块导出供 Serve/API 复用。
- ✅ Artifacts CLI 迁出：`ArtifactsArgs`/`cmd_artifacts` 与过滤、提取、摘要逻辑集中到 `src/cli/artifacts.rs`，`src/main.rs` 仅保留命令分派和共享的 `load_run_bundle`。 
- ✅ Tasks CLI 拆分：任务计划列表/展示命令已迁入 `src/cli/tasks.rs`，main 只负责分发，`TaskPlanStore` 交互集中在模块内。 
- ✅ Metrics CLI 拆分：指标相关 Args/handlers (`cmd_metrics`) 迁入 `src/cli/metrics.rs`，主文件仅保留指标计算逻辑 `compute_metrics_from_report`。 
- ✅ Schema CLI 拆分：`SchemaArgs`/`cmd_schema` 已迁入 `src/cli/schema.rs`，schema 校验逻辑通过模块复用，`src/main.rs` 不再承载这些子命令定义。 
- ✅ **去除未使用 Feature**：移除 `full-stack`/`legacy-tests`/`legacy-examples`/`soul-adapted` feature gates，旧示例与测试源码迁入 `docs/examples/legacy_code/`，Cargo 默认构建不再携带这些标志。 
- **重复工具整合**：集中 `tools/`、`automation/`、`export/` 中的相似逻辑（如 CSV/JSON 导出、计划执行）到共享库，减少多处复制；在模块头部补加“用途/状态”注释。 
- ✅ **test/示例压缩**：依赖旧 soul-base API 的 examples/tests 已迁入 `docs/examples/legacy_code/`（含源码与说明），默认仓库仅保留 Serve/API 相关示例与测试。 

## 3. 依赖与构建优化
- **依赖审计**：运行 `cargo udeps`/`cargo tree -d` 查找未使用或重复依赖，记录在本计划，并分两阶段移除：优先删无引用的 crate，再评估是否需要替换重型依赖（如 `openssl vendored`）。 
- ✅ **脚本清理**：`scripts/README.md` 记录了受支持的工具（clean_output、cleanup_profiles、perception_bench），并移除了重复的 `perception_benchmark.sh`，保留 sh/ps1 各 1 份，旧入口在 README 中标注迁移说明。 
- ✅ **配置示例瘦身**：`config/README.md` 说明现役 `.example` 文件和目录的用途（config.yaml、local.env、data_extract_profiles、plugins/permissions/policies/self_heal等），`config/archive/README.md` 则约定旧样例的归宿，防止顶层 `config/` 再次堆积遗留文件。 
- **CI/工具链**：若存在重复的 lint/test pipeline（例如 `ci/` + GitHub Actions + scripts/dev_checks.sh`），整合成一条标准流程，并在贡献指南中明确。

## 4. 开发流程与资产
- **输出目录管理**：默认将 `soulbrowser-output/*` 视为临时文件，提供一键清理脚本并在 `.gitignore` 注明；将必要的样例结果单独存入 `samples/`。✅ `scripts/clean_output.sh` / `.ps1` 已覆盖 `soulbrowser-output/`、`tmp/` 与 `plan*.json`。 
- **任务计划模板**：`plan*.json` 等示例仅保留最新格式，其余合并到 `docs/reference/`；说明如何生成/验证，避免多份半旧半新。 
- **版本标记**：对未完成或「在飞」的层级（L0/L1）在代码与文档中加 `MVP / In-flight / Legacy` 标签，提醒读者当前成熟度；完成后及时更新。 
- **审计/复用机制**：建立季度“精简审计” checklist（文档、依赖、示例、计划、脚本），在 release 前复查，防止冗余重新积累。

## 执行方式
1. 设立 “精简看板” 记录上述行动项及负责人，优先处理文档/示例的冗余，再逐步进入代码/依赖层。 
2. 每完成一项，在本文件中勾选或追加状态，并在 PR 模板中引用「Project Slimming Plan」。 
3. 一旦主要瘦身阶段完成，将关键决策追加到 README/Plans，并在贡献指南写明「如何避免冗余回归」。

> 完成定义：docs/README 指向的文档数量减少且无 404；`src/main.rs`/Serve 路由拆分；无未使用依赖；Legacy 示例/脚本标记清晰；季度审计机制上线。
