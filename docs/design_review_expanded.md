Design Review — SoulBrowser Core Design (Expanded)

目标与范围
- 汇总当前设计中的潜在风险、可落地的改进方向，以及对未来演进的落地方案。聚焦边界清晰性、租户隔离、安全默认态势、错误诊断、一致性、测试性与文档治理。

Executive Summary
- 优点：模块化强、覆盖多模态感知、策略治理、插件体系，具备良好的扩展性。 
- 主要风险：AppContext 的职责膨胀、租户缓存边界不清、安全默认态势不明确、错误边界不统一、Surface 生命周期耦合、测试覆盖不足、文档与实现不同步。
- 基于风险的优先改进方向：安全默认态势、统一错误边界、配置解析统一、租户边界与缓存策略、Surface 的生命周期隔离、Mock 测试提升、设计变更治理。

Architecture Overview (High Level)
- CLI Shell -> Kernel -> Surfaces (Serve, Gateway, Console, Perceive) -> Perception/Action|Registry/Scheduler -> State Center/Policy Center/Memory Center -> Gateways & Plugins
- 核心职责拆解（简述）
  - CLI: 参数解析、配置加载、调度入口。
  - Kernel: 运行时封装、租户 AppContext 构建、对 Chrome/CDP 的对接、网关路由构造。
  - Perception: 提供结构化、视觉、语义感知和输出的聚合（MultiModalPerception）。
  - Action/Scheduler/Registry: 动作执行、计划编排、事件流与状态管理。
  - Governance/Plugins/L7: 网关策略、权限、扩展接口、插件运行时。

Design Principles & Observability
- 接口清晰、职责分离、最小权限、可替换性、可观测性：日志、度量、事件、可追踪性。
- 配置驱动：环境变量+ YAML 配置+ CLI 参数三者协同工作，优先级需在文档中明确。
- 安全优先：默认应具备鉴权基线；观测数据应对策略变更可追溯。

Risks & Mitigations
1) AppContext 职责边界与缓存风险
- 风险：单一对象承担过多职责，缓存失效难以控制。 
- 缓解：拆分成独立组件/服务；缓存引入策略版本号、显式失效入口；租户隔离。

2) 安全默认态势
- 风险：Serve auth 未配置时暴露端口，生产环境风险。
- 缓解：强基线鉴权，提供显式开关；默认要有最小 token/白名单。

3) 配置可预测性
- 风险：配置来源/覆盖顺序混乱。
- 缓解：统一解析入口、明确优先级，提供 config lint/validate。

4) 错误处理一致性
- 风险：外部 API 错误多样化，排错困难。
- 缓解：统一错误类型（SoulBrowserError）外部暴露、内部保持灵活性。

5) Surfaces 与 Kernel 生命周期耦合
- 风险：高并发场景下资源竞争与清理复杂。
- 缓解：引入 Surface 级别生命周期边界、租户级别 AppContext、避免共享可变状态。

6) 测试与 Mock
- 风险：对 Chrome/CDP 的强依赖导致测试困难。
- 缓解：引入 Mock/Stub 路径，新增端到端测试用例与 CI 友好性。

7) 文档治理
- 风险：设计变更与实现不同步。
- 缓解：设计变更日志、契约文档、自动化文档同步。

改进优先级（建议排序）
- 最高优先：安全默认态势、统一错误边界、配置统一与校验。
- 中等优先：租户边界与缓存策略、Surface 生命周期隔离、测试覆盖升级。
- 长期：设计变更治理、文档自动化、契约化 API。

第一轮落地计划（示例，不涉及具体代码 patch）
- 计划目标：在不破坏现有探索性的前提下，先实现以下最小可行改动：
  1) 将 Serve 的认证基线改为显式可配置，默认必须提供 token，更新文档与示例。
  2) 引入统一错误边界：在 CLI/API 边界统一 SoulBrowserError，并确保错误路径可追溯。
  3) 引入配置解析统一入口并提供小型 lint/validate 命令。
  4) 提出 AppContext 的边界拆分方案（初步实现草案，如分离 Storage/Auth/ToolManager 的接口）。
  5) 增强测试能力：为感知/CDP 增设 Mock 路径，新增简单场景测试。

实施路线图与工作方式
- 采用逐步迭代：每次变更覆盖一个或两个紧耦合点，确保可回滚。 
- 文档与代码并行：每次变更须更新设计页的契约描述。
- 评估点：监控错误率、失败原因、认证相关的访问风险、测试覆盖率提升。

Appendix：关键文件与接口
- 设计边界与契约参考：docs/module_deep_dive.md, src/cli/app.rs, src/cli/runtime.rs, crates/soulbrowser-kernel/src/app_context.rs, crates/soulbrowser-kernel/src/kernel.rs, crates/soulbrowser-kernel/src/perception_service.rs
- 变更日志建议：docs/design_change_log.md（新建，用于记录重大设计变更）

如何参与
- 如你愿意，我可以把上述改动整理成一个可执行的 todowrite 计划，按优先级分解成具体任务（例如：A1、安全基线实现；A2、错误边界统一等），再逐步提交实现草案与测试用例。

