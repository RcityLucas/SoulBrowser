# L1 层完成路线图

**层级**: L1 统一内核层（Unified Kernel）  
**当前进度**: 80%  
**剩余工作**: 3 周  
**优先级**: P0/P1

---

## 📋 概述

L1 层的核心模块已完成，剩余 3 个关键功能需要实现：

1. **指标导出（Metrics Export）** - P0 优先级，1 周
2. **最小化重放（Minimal Replay）** - P1 优先级，1 周
3. **完整可观测性集成** - P1 优先级，1 周

---

## ✅ 已完成模块回顾

### 1. registry（注册中心）- 完成 ✅
- Session/Tab/Frame 生命周期管理
- 层级树结构维护
- 事件记录到 State Center
- 线程安全的状态访问

### 2. scheduler（调度器）- 完成 ✅
- ToolCall 验证、去重、优先级队列
- 与 Registry 集成
- 取消令牌支持
- CLI 命令：`soulbrowser scheduler`

### 3. state-center（状态中心）- 完成 ✅
- Ring buffers 事件存储
- 历史查询 API
- 调度结果跟踪
- 基础重放构建器（需完善）

### 4. policy-center（策略中心）- 完成 ✅
- 策略配置管理
- 运行时覆盖
- CLI 命令：`soulbrowser policy show/override`

### 5. event-bus（事件总线）- 完成 ✅
- 发布/订阅机制
- 跨模块消息传递

---

## 🎯 Week 1: 指标导出系统

### 优先级：P0 🔥

### 目标
实现 Prometheus 格式的指标导出，为生产监控提供支持。

### Day 1-2: Prometheus 集成与核心指标

**位置**: `crates/scheduler/src/metrics.rs`, `crates/registry/src/metrics.rs`

**任务清单**:

- [x] **添加依赖**
  ```toml
  [dependencies]
  prometheus = "0.13"
  lazy_static = "1.4"
  ```

- [x] **定义 Scheduler 指标**
  ```rust
  use prometheus::{IntCounter, IntGauge, Histogram, Registry};
  use lazy_static::lazy_static;
  
  lazy_static! {
      pub static ref SCHEDULER_QUEUE_LENGTH: IntGauge =
          IntGauge::new("scheduler_queue_length", "Current queue length").unwrap();
      
      pub static ref SCHEDULER_DISPATCHES_TOTAL: IntCounter =
          IntCounter::new("scheduler_dispatches_total", "Total dispatches").unwrap();
      
      pub static ref SCHEDULER_FAILURES_TOTAL: IntCounter =
          IntCounter::new("scheduler_failures_total", "Total failures").unwrap();
      
      pub static ref SCHEDULER_EXECUTION_DURATION: Histogram =
          Histogram::with_opts(
              HistogramOpts::new("scheduler_execution_duration_seconds", "Execution duration")
                  .buckets(vec![0.01, 0.05, 0.1, 0.5, 1.0, 5.0, 10.0])
          ).unwrap();
  }
  
  pub fn register_metrics(registry: &Registry) {
      registry.register(Box::new(SCHEDULER_QUEUE_LENGTH.clone())).unwrap();
      registry.register(Box::new(SCHEDULER_DISPATCHES_TOTAL.clone())).unwrap();
      registry.register(Box::new(SCHEDULER_FAILURES_TOTAL.clone())).unwrap();
      registry.register(Box::new(SCHEDULER_EXECUTION_DURATION.clone())).unwrap();
  }
  ```

- [x] **定义 Registry 指标**
  ```rust
  lazy_static! {
      pub static ref REGISTRY_SESSIONS_TOTAL: IntGauge =
          IntGauge::new("registry_sessions_total", "Total sessions").unwrap();
      
      pub static ref REGISTRY_PAGES_ACTIVE: IntGauge =
          IntGauge::new("registry_pages_active", "Active pages").unwrap();
      
      pub static ref REGISTRY_FRAMES_TOTAL: IntGauge =
          IntGauge::new("registry_frames_total", "Total frames").unwrap();
  }
  ```

- [x] **集成到 Scheduler**
  ```rust
  impl Scheduler {
      pub async fn dispatch(&self, tool_call: ToolCall) -> Result<()> {
          // 更新队列长度
          SCHEDULER_QUEUE_LENGTH.inc();
          
          let start = Instant::now();
          
          match self.execute_tool_call(tool_call).await {
              Ok(_) => {
                  SCHEDULER_DISPATCHES_TOTAL.inc();
              }
              Err(e) => {
                  SCHEDULER_FAILURES_TOTAL.inc();
              }
          }
          
          // 记录执行时间
          let duration = start.elapsed().as_secs_f64();
          SCHEDULER_EXECUTION_DURATION.observe(duration);
          
          SCHEDULER_QUEUE_LENGTH.dec();
          
          Ok(())
      }
  }
  ```

**验收标准**:
- ✅ 所有指标正确定义
- ✅ 指标正确更新
- ✅ 单元测试通过

---

### Day 3-4: HTTP 指标端点 + 自定义指标

**位置**: `src/main.rs`, `crates/cdp-adapter/src/metrics.rs`

**任务清单**:

- [x] **HTTP 服务器**
  ```toml
  [dependencies]
  axum = "0.7"
  tokio = { version = "1.39", features = ["full"] }
  ```

  ```rust
  use axum::{routing::get, Router};
  use prometheus::{Encoder, TextEncoder};
  
  async fn metrics_handler() -> String {
      let encoder = TextEncoder::new();
      let metric_families = prometheus::gather();
      let mut buffer = vec![];
      encoder.encode(&metric_families, &mut buffer).unwrap();
      String::from_utf8(buffer).unwrap()
  }
  
  pub async fn start_metrics_server(port: u16) {
      let app = Router::new().route("/metrics", get(metrics_handler));
      
      let addr = format!("0.0.0.0:{}", port);
      tracing::info!("Metrics server listening on {}", addr);
      
      axum::Server::bind(&addr.parse().unwrap())
          .serve(app.into_make_service())
          .await
          .unwrap();
  }
  ```

- [x] **CDP Adapter 指标**
  ```rust
  lazy_static! {
      pub static ref CDP_COMMANDS_TOTAL: IntCounterVec =
          IntCounterVec::new(
              Opts::new("cdp_commands_total", "Total CDP commands"),
              &["command"]
          ).unwrap();
      
      pub static ref CDP_COMMAND_DURATION: HistogramVec =
          HistogramVec::new(
              HistogramOpts::new("cdp_command_duration_seconds", "CDP command duration")
                  .buckets(vec![0.01, 0.05, 0.1, 0.5, 1.0, 5.0]),
              &["command"]
          ).unwrap();
      
      pub static ref CDP_RECONNECTIONS_TOTAL: IntCounter =
          IntCounter::new("cdp_reconnections_total", "Total reconnections").unwrap();
  }
  
  impl CdpAdapter {
      pub async fn send_command(&self, method: &str, params: Value) -> Result<Value> {
          let start = Instant::now();
          
          let result = self.transport.send_command(method, params).await;
          
          // 记录指标
          CDP_COMMANDS_TOTAL.with_label_values(&[method]).inc();
          CDP_COMMAND_DURATION
              .with_label_values(&[method])
              .observe(start.elapsed().as_secs_f64());
          
          result
      }
      
      pub async fn reconnect(&mut self) -> Result<()> {
          CDP_RECONNECTIONS_TOTAL.inc();
          // ... reconnect logic
      }
  }
  ```

- [x] **CLI 集成**
  ```rust
  // 在 main.rs 中启动
  #[tokio::main]
  async fn main() {
      // ... 初始化
      
      // 注册所有指标
      let registry = prometheus::Registry::new();
      scheduler::metrics::register_metrics(&registry);
      registry::metrics::register_metrics(&registry);
      cdp_adapter::metrics::register_metrics(&registry);
      
      // 启动指标服务器
      tokio::spawn(async move {
          start_metrics_server(9090).await;
      });
      
      // ... 主逻辑
  }
  ```

**验收标准**:
- ✅ HTTP 端点 `/metrics` 可访问
- ✅ 所有指标正确导出
- ✅ Prometheus 可正确抓取

---

### Day 5: 性能基线与基准测试

**位置**: `benches/scheduler_bench.rs`

**任务清单**:

- [ ] **添加 Criterion**
  ```toml
  [dev-dependencies]
  criterion = "0.5"
  
  [[bench]]
  name = "scheduler_bench"
  harness = false
  ```

- [ ] **调度器基准测试**
  ```rust
  use criterion::{black_box, criterion_group, criterion_main, Criterion};
  
  fn scheduler_dispatch_benchmark(c: &mut Criterion) {
      let rt = tokio::runtime::Runtime::new().unwrap();
      let scheduler = rt.block_on(async {
          Scheduler::new().await
      });
      
      c.bench_function("scheduler_dispatch", |b| {
          b.to_async(&rt).iter(|| async {
              let tool_call = ToolCall {
                  id: "test".to_string(),
                  tool: "navigate".to_string(),
                  params: json!({ "url": "https://example.com" }),
              };
              scheduler.dispatch(black_box(tool_call)).await.unwrap();
          });
      });
  }
  
  criterion_group!(benches, scheduler_dispatch_benchmark);
  criterion_main!(benches);
  ```

- [ ] **运行基准测试**
  ```bash
  cargo bench
  
  # 生成报告
  open target/criterion/report/index.html
  ```

- [ ] **记录基线**
  ```markdown
  ## Performance Baseline (2025-01-21)
  
  ### Scheduler
  - dispatch: 45μs (P50), 120μs (P95), 250μs (P99)
  - queue_length: 0-100 稳定
  
  ### Registry
  - resolve_route: 5μs (P50), 15μs (P95)
  
  ### CDP Adapter
  - navigate: 450ms (P50), 850ms (P95)
  - click: 85ms (P50), 180ms (P95)
  - type_text: 120ms (P50), 250ms (P95)
  ```

**验收标准**:
- ✅ 基准测试可重复运行
- ✅ 性能基线已记录
- ✅ P95/P99 在可接受范围

---

## 🎯 Week 2: 最小化重放功能

### 优先级：P1

### 目标
从 State Center 提取事件，生成可重放的时间线，用于问题诊断和调试。

### Day 1-2: 重放数据结构

**位置**: `crates/state-center/src/replay.rs`

**任务清单**:

- [ ] **重放数据结构**
  ```rust
  use serde::{Serialize, Deserialize};
  use chrono::{DateTime, Utc};
  
  #[derive(Debug, Serialize, Deserialize)]
  pub struct ReplayTimeline {
      pub session_id: String,
      pub started_at: DateTime<Utc>,
      pub finished_at: DateTime<Utc>,
      pub events: Vec<ReplayEvent>,
      pub metadata: ReplayMetadata,
  }
  
  #[derive(Debug, Serialize, Deserialize)]
  pub struct ReplayEvent {
      pub offset_ms: u64,  // 相对 started_at 的偏移
      pub event_type: String,
      pub data: serde_json::Value,
  }
  
  #[derive(Debug, Serialize, Deserialize)]
  pub struct ReplayMetadata {
      pub tool_calls: Vec<String>,
      pub pages_visited: Vec<String>,
      pub errors: Vec<String>,
      pub total_duration_ms: u64,
  }
  ```

- [ ] **序列化格式**
  ```toml
  [dependencies]
  bincode = "1.3"
  flate2 = "1.0"
  ```

  ```rust
  pub fn serialize_timeline(timeline: &ReplayTimeline) -> Result<Vec<u8>> {
      use flate2::write::GzEncoder;
      use flate2::Compression;
      
      // 先用 bincode 序列化
      let encoded = bincode::serialize(timeline)?;
      
      // 再用 gzip 压缩
      let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
      std::io::Write::write_all(&mut encoder, &encoded)?;
      let compressed = encoder.finish()?;
      
      Ok(compressed)
  }
  
  pub fn deserialize_timeline(data: &[u8]) -> Result<ReplayTimeline> {
      use flate2::read::GzDecoder;
      
      // 解压
      let mut decoder = GzDecoder::new(data);
      let mut decompressed = Vec::new();
      std::io::Read::read_to_end(&mut decoder, &mut decompressed)?;
      
      // 反序列化
      let timeline: ReplayTimeline = bincode::deserialize(&decompressed)?;
      Ok(timeline)
  }
  ```

**验收标准**:
- ✅ 数据结构定义完整
- ✅ 序列化/反序列化正确
- ✅ 压缩率 > 50%

---

### Day 3-4: 重放构建器

**位置**: `crates/state-center/src/replay.rs`

**任务清单**:

- [ ] **重放构建器**
  ```rust
  pub struct ReplayBuilder {
      state_center: Arc<StateCenter>,
  }
  
  impl ReplayBuilder {
      pub async fn build_timeline(
          &self,
          session_id: &str,
      ) -> Result<ReplayTimeline> {
          // 提取所有事件
          let events = self.state_center.query_history(EventFilter {
              session_id: Some(session_id.to_string()),
              ..Default::default()
          }).await?;
          
          if events.is_empty() {
              return Err(ReplayError::NoEvents);
          }
          
          // 计算时间偏移
          let started_at = events[0].timestamp;
          let finished_at = events.last().unwrap().timestamp;
          
          let replay_events: Vec<ReplayEvent> = events.iter()
              .filter(|e| self.should_include(e))  // 过滤噪音
              .map(|e| ReplayEvent {
                  offset_ms: (e.timestamp - started_at).num_milliseconds() as u64,
                  event_type: e.event_type.clone(),
                  data: self.sanitize_data(&e.data),  // 脱敏
              })
              .collect();
          
          // 生成元数据
          let metadata = self.build_metadata(&events);
          
          Ok(ReplayTimeline {
              session_id: session_id.to_string(),
              started_at,
              finished_at,
              events: replay_events,
              metadata,
          })
      }
      
      fn should_include(&self, event: &StateEvent) -> bool {
          // 过滤规则
          match event.event_type.as_str() {
              "HEARTBEAT" => false,  // 跳过心跳
              "METRICS" => false,    // 跳过指标
              _ => true,
          }
      }
      
      fn sanitize_data(&self, data: &Value) -> Value {
          // 脱敏处理
          let mut sanitized = data.clone();
          
          // 移除敏感字段
          if let Some(obj) = sanitized.as_object_mut() {
              obj.remove("password");
              obj.remove("token");
              obj.remove("cookie");
              
              // URL 查询参数打码
              if let Some(url) = obj.get_mut("url") {
                  if let Some(url_str) = url.as_str() {
                      *url = json!(redact_url_params(url_str));
                  }
              }
          }
          
          sanitized
      }
      
      fn build_metadata(&self, events: &[StateEvent]) -> ReplayMetadata {
          let mut tool_calls = Vec::new();
          let mut pages_visited = Vec::new();
          let mut errors = Vec::new();
          
          for event in events {
              match event.event_type.as_str() {
                  "DISPATCH_STARTED" => {
                      if let Some(tool) = event.data.get("tool") {
                          tool_calls.push(tool.as_str().unwrap().to_string());
                      }
                  }
                  "PAGE_LOADED" => {
                      if let Some(url) = event.data.get("url") {
                          pages_visited.push(url.as_str().unwrap().to_string());
                      }
                  }
                  "DISPATCH_FAILED" => {
                      if let Some(error) = event.data.get("error") {
                          errors.push(error.as_str().unwrap().to_string());
                      }
                  }
                  _ => {}
              }
          }
          
          let total_duration_ms = if events.len() > 1 {
              (events.last().unwrap().timestamp - events[0].timestamp)
                  .num_milliseconds() as u64
          } else {
              0
          };
          
          ReplayMetadata {
              tool_calls,
              pages_visited,
              errors,
              total_duration_ms,
          }
      }
  }
  ```

**验收标准**:
- ✅ 事件正确提取
- ✅ 噪音正确过滤
- ✅ 敏感数据脱敏
- ✅ 元数据正确生成

---

### Day 5: CLI 命令集成

**位置**: `src/main.rs`

**任务清单**:

- [ ] **导出命令**
  ```rust
  #[derive(Parser)]
  #[command(name = "replay")]
  #[command(about = "Replay management")]
  struct ReplayArgs {
      #[command(subcommand)]
      command: ReplayCommand,
  }
  
  #[derive(Subcommand)]
  enum ReplayCommand {
      Export {
          session_id: String,
          #[arg(short, long)]
          output: Option<PathBuf>,
      },
      View {
          replay_file: PathBuf,
      },
  }
  
  async fn handle_replay_command(args: ReplayArgs) -> Result<()> {
      match args.command {
          ReplayCommand::Export { session_id, output } => {
              let builder = ReplayBuilder::new(state_center);
              let timeline = builder.build_timeline(&session_id).await?;
              
              let data = serialize_timeline(&timeline)?;
              
              let output_path = output.unwrap_or_else(|| {
                  PathBuf::from(format!("replay_{}.bin.gz", session_id))
              });
              
              std::fs::write(&output_path, data)?;
              println!("Replay exported to: {}", output_path.display());
              
              Ok(())
          }
          ReplayCommand::View { replay_file } => {
              let data = std::fs::read(&replay_file)?;
              let timeline = deserialize_timeline(&data)?;
              
              println!("Session ID: {}", timeline.session_id);
              println!("Started at: {}", timeline.started_at);
              println!("Duration: {}ms", timeline.metadata.total_duration_ms);
              println!("\nTool Calls:");
              for tool in &timeline.metadata.tool_calls {
                  println!("  - {}", tool);
              }
              println!("\nPages Visited:");
              for url in &timeline.metadata.pages_visited {
                  println!("  - {}", url);
              }
              println!("\nEvents: {}", timeline.events.len());
              
              for event in timeline.events.iter().take(10) {
                  println!("  [{:6}ms] {}", event.offset_ms, event.event_type);
              }
              
              Ok(())
          }
      }
  }
  ```

- [ ] **使用示例**
  ```bash
  # 导出重放
  soulbrowser replay export abc123 --output session.replay
  
  # 查看重放
  soulbrowser replay view session.replay
  
  # 输出:
  # Session ID: abc123
  # Started at: 2025-01-21 10:30:00 UTC
  # Duration: 45230ms
  # 
  # Tool Calls:
  #   - navigate
  #   - click
  #   - type_text
  # 
  # Pages Visited:
  #   - https://example.com
  #   - https://example.com/login
  # 
  # Events: 156
  #   [     0ms] DISPATCH_STARTED
  #   [   450ms] PAGE_LOADED
  #   [  1200ms] DISPATCH_FINISHED
  #   ...
  ```

**验收标准**:
- ✅ CLI 命令正常工作
- ✅ 导出文件可读取
- ✅ 查看输出友好

---

## 🎯 Week 3: 完整可观测性集成

### 优先级：P1

### 目标
集成 tracing，实现结构化日志和可选的外部导出。

### Day 1-2: Tracing 集成

**位置**: 全局集成

**任务清单**:

- [x] **添加依赖**
  ```toml
  [dependencies]
  tracing = "0.1"
  tracing-subscriber = { version = "0.3", features = ["env-filter", "json"] }
  tracing-appender = "0.2"
  ```

- [ ] **初始化 tracing**
  ```rust
  use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};
  
  fn init_tracing() {
      tracing_subscriber::registry()
          .with(EnvFilter::from_default_env()
              .add_directive("soulbrowser=debug".parse().unwrap()))
          .with(tracing_subscriber::fmt::layer()
              .with_target(true)
              .with_thread_ids(true))
          .init();
  }
  ```

- [ ] **Span 设计**
  ```rust
  // Session span
  #[instrument(name = "session", skip(self), fields(session_id = %session_id))]
  pub async fn create_session(&self) -> SessionId {
      let session_id = SessionId::new();
      tracing::info!("Session created");
      session_id
  }
  
  // Page span
  #[instrument(name = "page", parent = session_span, fields(page_id = %page_id))]
  pub async fn create_page(&self, session_id: SessionId) -> PageId {
      let page_id = PageId::new();
      tracing::info!("Page created");
      page_id
  }
  
  // Action span
  #[instrument(name = "action", skip(self), fields(action_id = %ctx.action_id, tool = %tool_name))]
  pub async fn execute_action(&self, ctx: &ExecCtx, tool_name: &str) -> Result<ActionReport> {
      tracing::debug!("Action started");
      // ...
      tracing::info!(latency_ms = %report.latency_ms, "Action finished");
      Ok(report)
  }
  
  // Primitive span
  #[instrument(name = "primitive", parent = action_span, fields(primitive = "click"))]
  pub async fn click(&self, ctx: &ExecCtx, anchor: &AnchorDescriptor) -> Result<ActionReport> {
      tracing::trace!("Click primitive executing");
      // ...
  }
  ```

**验收标准**:
- ✅ Tracing 正确初始化
- ✅ Span 层级正确
- ✅ Context 传播正常

---

### Day 3-4: 结构化日志

**位置**: 全局

**任务清单**:

- [ ] **JSON 格式日志**
  ```rust
  fn init_json_logging() {
      let file_appender = tracing_appender::rolling::daily("logs", "soulbrowser.log");
      let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);
      
      tracing_subscriber::registry()
          .with(EnvFilter::from_default_env())
          .with(
              tracing_subscriber::fmt::layer()
                  .json()
                  .with_writer(non_blocking)
          )
          .init();
  }
  ```

- [ ] **敏感数据脱敏**
  ```rust
  use tracing::field::{Field, Visit};
  
  struct SanitizingVisitor;
  
  impl Visit for SanitizingVisitor {
      fn record_str(&mut self, field: &Field, value: &str) {
          let sanitized = match field.name() {
              "password" | "token" | "cookie" => "***REDACTED***",
              "url" => redact_url_params(value),
              _ => value,
          };
          // ... 记录
      }
  }
  ```

- [ ] **日志轮转配置**
  ```rust
  // 按大小轮转
  let file_appender = tracing_appender::rolling::RollingFileAppender::new(
      tracing_appender::rolling::Rotation::DAILY,
      "logs",
      "soulbrowser.log",
  );
  ```

**验收标准**:
- ✅ JSON 日志格式正确
- ✅ 敏感数据脱敏
- ✅ 日志轮转正常

---

### Day 5: 可选的外部导出

**位置**: `src/observability.rs`

**任务清单**:

- [ ] **Jaeger Exporter（可选）**
  ```toml
  [dependencies]
  opentelemetry = { version = "0.21", optional = true }
  opentelemetry-jaeger = { version = "0.20", optional = true }
  tracing-opentelemetry = { version = "0.22", optional = true }
  
  [features]
  jaeger = ["opentelemetry", "opentelemetry-jaeger", "tracing-opentelemetry"]
  ```

  ```rust
  #[cfg(feature = "jaeger")]
  fn init_jaeger() -> Result<()> {
      use opentelemetry::global;
      use opentelemetry_jaeger::Exporter;
      
      let tracer = Exporter::builder()
          .with_agent_endpoint("localhost:6831")
          .init()?;
      
      global::set_tracer_provider(tracer);
      
      Ok(())
  }
  ```

**验收标准**:
- ✅ Feature flag 正常工作
- ✅ Jaeger 可正确接收

---

## �� 验收标准

### 功能验收
- ✅ 指标导出支持 Prometheus 格式
- ✅ HTTP `/metrics` 端点可访问
- ✅ 重放功能可生成完整时间线
- ✅ CLI 命令全部可用
- ✅ 所有模块有 tracing span
- ✅ 结构化日志正确输出

### 性能验收
- ✅ 指标更新开销 < 1μs
- ✅ 重放生成时间 < 1s（1000 事件）
- ✅ 日志写入不阻塞主线程

### 质量验收
- ✅ 单元测试覆盖率 > 80%
- ✅ 集成测试全部通过
- ✅ 文档完整

---

## 🚀 交付物

### Week 1 交付
- Prometheus 指标导出系统
- HTTP `/metrics` 端点
- 性能基线报告

### Week 2 交付
- 重放数据结构与序列化
- CLI `replay export/view` 命令
- 重放示例文件

### Week 3 交付
- Tracing 集成
- 结构化日志系统
- 可观测性文档

---

**文档维护**: 每周更新进度。
