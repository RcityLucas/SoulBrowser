# Serve/API 优化计划

> 目标：在 `cargo run --bin soulbrowser -- --metrics-port 0 serve --port <PORT>` 模式下，使 Web Console / REST API 更易扩展、更稳定、可观测性更强，同时为多租户、真实浏览器池化和 Planner 执行提供可持续的演进路径。

## 1. 顶层 Serve / API 外壳
- ✅ **配置拆分**：`config/config.yaml` 新增 `serve.*` 段落，Serve CLI 会按 `CLI > config > env` 的优先级解析 `ws_url`、LLM 缓存目录、共享 session toggle 以及每分钟速率限制，同时保留 `SOUL_SERVE_WS_URL`/`SOULBROWSER_LLM_CACHE_DIR` 等 env 覆盖，便于按环境热切换。 
- **路由模块化**：把 `build_console_router` 拆成 perception/task/memory/plugin 子路由文件，支持按需挂载 & 未来 Gateway 复用。 
- **健康探针**：新增 `/readyz`、`/livez`、`/metrics`（复用 `metrics::spawn_metrics_server`）并在 `cmd_serve` 启动时做 Chrome/WebSocket 探活。 
- **速率限制扩展**：为 `RateLimiter` 提供 Redis/外部存储实现，或至少增加分片/清理逻辑，保证多实例部署的一致性。✅ Serve 模式默认每 60s 清理一次闲置 bucket（`SOUL_RATE_LIMIT_BUCKET_TTL_SECS`/`SOUL_RATE_LIMIT_GC_SECS` 可调），避免长时运行吃光内存并为未来外部化铺路。 

## 2. AppContext & 全局服务
- **多租户**：允许 `get_or_create_context` 根据 tenant/topic 维护多份 `AppContext`，or 支持强制刷新，避免不同 policy/output 互相污染。✅ Serve 内部改为持有可热切换的 `AppContext` 句柄并暴露 `/api/admin/context/refresh`，支持在线刷新 tenant context（自动重启 Memory/Plugin/Self-Heal metrics 任务）。 
- **初始化观测**：在 Scheduler、Registry、L0Bridge 初始化过程中打 `tracing` span +耗时日志，失败时自动重试+退避。✅ `AppContext::new` 现为 Registry/Scheduler/L0Bridge 的创建记录耗时日志，并在默认 session/page 初始化失败时执行指数退避重试，便于排查初始化抖动。 
- **定时任务治理**：Memory/Self-Heal/Plugin 指标刷新改为可配置 interval，并返回 `JoinHandle`，便于在 Serve 停止时优雅退出。✅ `AppContext` 统一用可取消后台任务托管 Memory/Self-Heal/Plugin 指标刷新（`SOUL_*_METRICS_INTERVAL_SECS` 可调），暴露 `shutdown_background_tasks()`，Serve 关闭时可主动收束任务。
- ✅ **持久化兜底**：MemoryCenter 失败时通过 `tracing` 打出明确原因，并新增 `/api/admin/memory/persist` 触发手动重试；StateCenter snapshot 日志带上路径/ join error 详情，方便排查。 

## 3. 感知管线（PerceptionService）
- **CDP adapter 完成度**：补足真实 CDP 连接、错误恢复、profile 清理 & Chrome crash 自愈；`perceive_with_shared` 失效时自动回退到临时实例。 
- **动态池化策略**：基于 `metrics_snapshot`（shared hit/miss/avg_duration_ms）设置阈值，低命中自动降级，命中高时才复用；指标经 Prometheus 暴露给 `/api/perceive/metrics`。✅ `PerceptionService` 增加动态池控（`SOUL_PERCEPTION_POOL_*`）与健康/指标输出，命中率过低或平均时延异常时自动冷却共享 session，并在命中率恢复后再启用。 
- **上下文采集限流**：`capture_chat_context_snapshot` 增加并发阈值，避免 `/api/chat` 因附带感知而阻塞。 
- ✅ **可配置感知参数**：`/api/perceive` 支持 `viewport`、`cookies`、`inject_script` 字段，Server 会在执行前设置设备指标、注入 Cookie 并在 DOM Ready 后执行自定义脚本（自动跳过共享会话以免污染）。 

## 4. Planner / 执行流
- **工具别名统一**：在 `ChatRunner::normalize_custom_tools` 中自动同步插件注册表 alias，确保 planner 输出标准化（`data.extract-site`, `data.deliver.structured` 等）。 
- **LLM 缓存分区**：Serve CLI `--llm-cache-dir` 支持多模型/多租户前缀，提供缓存命中指标。 
- **执行指标**：`execute_plan` / Scheduler dispatcher 需要把 per-step latency、retry、error code 写入 Task Center + Prometheus，Web Console 直观展示。 
- **SSE 优化**：`TaskStatusRegistry` 推送支持批量/快照，降低高并发锁争用。 

## 5. Task Center & 存储
- **Plan/Artifact 生命周期**：`TaskPlanStore` / `soulbrowser-output` 引入 TTL、压缩/归档策略和清理脚本；Artifacts 支持预签名下载或后台异步清理。 
- **断线恢复**：`task_stream_handler` 支持 `Last-Event-ID` 与历史重放，前端 Web Console 加断线自动重连。 
- **日志分页**：`/api/tasks/:id/logs` 支持 cursor/pagination，避免一次返回过多数据。 

## 6. 可观测性 & 安全
- **Tracing/metrics 统一**：对 perception/chat/scheduler/memory/self-heal 路径打 span、export 指标，统一 `soul.*` 命名。 
- **默认鉴权**：Serve 模式默认启用 `SOUL_STRICT_AUTHZ` + token/IP 白名单（沿用 Gateway 适配器），CLI 提供 override。 
- **故障注入**：利用 `SelfHealRegistry`/Watchdog，在测试环境中注入失败并验证自愈/告警路径，更新 `docs/reference/SECURITY_NOTES.md`。 

> 以上计划自顶向下覆盖 Serve 外壳 → AppContext → 感知/Planner → Task Center → 可观测性，可逐项排期并在 PR 模板中链接本计划，确保执行透明。
