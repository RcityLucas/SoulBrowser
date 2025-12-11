# Serve/API 优化计划

> 目标：在 `cargo run --bin soulbrowser -- --metrics-port 0 serve --port <PORT>` 模式下，使 Web Console / REST API 更易扩展、更稳定、可观测性更强，同时为多租户、真实浏览器池化和 Planner 执行提供可持续的演进路径。

## 1. 顶层 Serve / API 外壳
- ✅ **配置拆分**：`config/config.yaml` 新增 `serve.*` 段落，Serve CLI 会按 `CLI > config > env` 的优先级解析 `ws_url`、LLM 缓存目录、共享 session toggle 以及每分钟速率限制，同时保留 `SOUL_SERVE_WS_URL`/`SOULBROWSER_LLM_CACHE_DIR` 等 env 覆盖，便于按环境热切换。 
- ✅ **路由模块化**：`build_console_router` 基于 `ServeRouterModules` 装配 perception/task/memory/plugin 等子路由（`router::perception_routes()` 等函数可单独复用），按需挂载支持 Gateway/自定义 Serve 变体。 
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
- ✅ **上下文采集限流**：`capture_chat_context_snapshot` 通过 `SOUL_CHAT_CONTEXT_LIMIT` + `SOUL_CHAT_CONTEXT_WAIT_MS` 控制并发和等待时间，超过阈值直接降级返回错误（由 `/api/chat` 捕获到 metadata），防止请求挂起。 
- ✅ **可配置感知参数**：`/api/perceive` 支持 `viewport`、`cookies`、`inject_script` 字段，Server 会在执行前设置设备指标、注入 Cookie 并在 DOM Ready 后执行自定义脚本（自动跳过共享会话以免污染）。 

## 4. Planner / 执行流
- ✅ **工具别名统一**：`ChatRunner::normalize_custom_tools` 会把 `page.observe`、`github.extract-repo`、`data.deliver.json` 等历史别名重写为 `data.extract-site` / `data.parse.*` / `data.deliver.structured`，计划和 Flow 定义保持标准化。 
- ✅ **LLM 缓存分区**：Serve CLI `--llm-cache-dir` 支持多模型/多租户前缀，命名空间标签（tenant + provider + model）会通过 `soul_llm_cache_events_total{namespace,event}` 暴露命中/未命中/错误指标，方便 Grafana/Prometheus 观察缓存质量。 
- ✅ **执行指标**：`execute_plan` 现会为每个步骤写回等待/执行耗时与重试次数，`TaskStatusRegistry` 推送 `AgentHistoryEntry`（含 tool_kind / wait_ms / run_ms）且 Web Console 展示细节，同时通过 `soul_execution_step_latency_ms`、`soul_execution_step_attempts_total` 暴露 Prometheus 指标。 
- ✅ **SSE 优化**：`TaskStatusRegistry` 发送批量事件（观察、Overlay 等）时只持有一次锁并一次广播，降低高并发下的锁争用，History buffer 仍支持 `Last-Event-ID` 重放。 

## 5. Task Center & 存储
- ✅ **Plan/Artifact 生命周期**：`TaskPlanStore` 在 Serve 启动时依据 `SOUL_PLAN_TTL_DAYS` 清理过期计划；`soulbrowser-output/tasks/<task_id>` 目录也会根据 `SOUL_OUTPUT_TTL_DAYS` 自动移除旧的执行快照，避免输出目录无限增长。后续仍可补充压缩/归档策略与异步下载。 
- ✅ **断线恢复**：`/api/tasks/:id/events` SSE 支持 `Last-Event-ID`/`cursor` 回放，TypeScript SDK 提供 `client.streamTaskEvents()` 自动重连并暴露 `lastId`，控制台/脚本断线后可凭游标补齐事件。 
- ✅ **日志分页**：`/api/tasks/:id/logs` 现支持 `limit`/`cursor`/`since`（RFC3339）筛选并返回 `next_cursor`，用于 Web Console 断线后增量补齐任务日志。 

## 6. 可观测性 & 安全
- ✅ **Tracing/metrics 统一**：对 perception/chat/tasks/memory/self-heal 路径打 span、export 指标，统一 `soul.*` 命名，Prometheus 也暴露 LLM 缓存/执行指标，方便在 Grafana 搜索 `soul.*` 事件。 
- ✅ **默认鉴权**：Serve 模式默认启用 `SOUL_STRICT_AUTHZ` + token/IP 白名单，`/` 等静态页面仅做 IP 校验，`/api/**` 则使用 `GatewayPolicy` token 校验（`x-soulbrowser-token`/`Authorization`/浏览器本地保存的 token）。控制台首页新增 token 表单并自动在请求里附带凭证，CLI 仍可通过 `--disable-auth` 或 `--allow-ip/--auth-token` 覆盖。 
- ✅ **故障注入**：`/api/self-heal/strategies/:id/inject` 可以触发 `SelfHealRegistry` 事件（记录在 webhook/日志），便于在测试环境中模拟策略告警/自愈路径；更新 `docs/reference/SECURITY_NOTES.md` 时可引用此端点。 

> 以上计划自顶向下覆盖 Serve 外壳 → AppContext → 感知/Planner → Task Center → 可观测性，可逐项排期并在 PR 模板中链接本计划，确保执行透明。
