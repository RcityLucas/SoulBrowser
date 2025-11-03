# L0 层详细实施路线图

**层级**: L0 运行与适配层（Runtime & Adapters）  
**当前进度**: 40%  
**预计完成时间**: 6-8 周  
**优先级**: P0（最高）

---

## 📋 概述

L0 层是 SoulBrowser 的基础设施层，负责与浏览器的直接交互。包含 5 个核心模块：

1. **cdp-adapter** - Chrome DevTools Protocol 适配器
2. **permissions-broker** - 权限管理代理
3. **network-tap-light** - 轻量级网络监控
4. **stealth** - 反检测与隐身功能
5. **extensions-bridge** - 浏览器扩展桥接

**关键阻塞关系**: CDP Adapter 是其他模块的基础，必须优先完成。

---

## 🎯 Milestone 1: CDP Adapter 核心（3-4 周）

### 优先级：P0 🔥

### Week 1-2: 传输层与连接管理

#### Day 1-3: ChromiumTransport 实现

**位置**: `crates/cdp-adapter/src/transport.rs`

**任务清单**:
- [ ] 集成 `chromiumoxide` crate
  ```toml
  [dependencies]
  chromiumoxide = "0.5"
  chromiumoxide_cdp = "0.5"
  ```
  
- [ ] 实现 `ChromiumTransport` 结构体
  ```rust
  pub struct ChromiumTransport {
      browser: Browser,
      handler: Handler,
      command_tx: mpsc::Sender<CommandRequest>,
      event_rx: mpsc::Receiver<TransportEvent>,
      inflight: Arc<DashMap<u64, oneshot::Sender<Value>>>,
  }
  ```

- [ ] 浏览器启动逻辑
  - 读取环境变量 `SOULBROWSER_CHROME`
  - 回退到系统 PATH
  - 回退到 chromiumoxide 自动检测
  - 启动参数配置：
    ```rust
    let args = vec![
        "--disable-blink-features=AutomationControlled",
        "--disable-dev-shm-usage",
        "--no-sandbox",  // 可选，根据策略
    ];
    ```

- [ ] WebSocket 连接建立
  ```rust
  let (browser, mut handler) = Browser::launch(
      BrowserConfig::builder()
          .chrome_executable(chrome_path)
          .args(args)
          .build()?
  ).await?;
  ```

- [ ] 连接既有浏览器实例（可选）
  ```rust
  let browser = Browser::connect(ws_url).await?;
  ```

**验收标准**:
- ✅ 成功启动 Chrome/Chromium
- ✅ WebSocket 连接建立
- ✅ 环境变量配置生效
- ✅ 错误处理完善（浏览器不存在、端口占用等）

---

#### Day 4-5: 事件循环实现

**位置**: `crates/cdp-adapter/src/transport.rs`

**任务清单**:
- [ ] 实现 `start()` 事件循环
  ```rust
  async fn start(&mut self) -> Result<()> {
      loop {
          tokio::select! {
              Some(event) = self.handler.next() => {
                  self.handle_cdp_event(event).await?;
              }
              Some(cmd) = self.command_rx.recv() => {
                  self.handle_command(cmd).await?;
              }
              else => break,
          }
      }
      Ok(())
  }
  ```

- [ ] Inflight 请求映射
  ```rust
  pub struct CommandRequest {
      pub id: u64,
      pub method: String,
      pub params: Value,
      pub response_tx: oneshot::Sender<Value>,
  }
  
  // 存储
  self.inflight.insert(cmd.id, cmd.response_tx);
  
  // 匹配响应
  if let Some((_, tx)) = self.inflight.remove(&response_id) {
      let _ = tx.send(response_value);
  }
  ```

- [ ] `next_event()` 返回 TransportEvent
  ```rust
  async fn next_event(&mut self) -> Option<TransportEvent> {
      if let Some(event) = self.event_rx.recv().await {
          Some(event)
      } else {
          None
      }
  }
  ```

**验收标准**:
- ✅ 事件循环正常运行
- ✅ 请求/响应正确匹配
- ✅ 并发请求处理正确
- ✅ 无内存泄漏（inflight 清理）

---

#### Day 6-7: 命令发送机制

**位置**: `crates/cdp-adapter/src/transport.rs`

**任务清单**:
- [ ] 实现 `send_command()`
  ```rust
  async fn send_command(
      &self,
      method: &str,
      params: Value,
      timeout: Duration,
  ) -> Result<Value, AdapterError> {
      let id = self.next_command_id();
      let (tx, rx) = oneshot::channel();
      
      let cmd = CommandRequest { id, method, params, response_tx: tx };
      self.command_tx.send(cmd).await?;
      
      // 超时等待
      match tokio::time::timeout(timeout, rx).await {
          Ok(Ok(value)) => Ok(value),
          Ok(Err(_)) => Err(AdapterError::Internal("channel closed")),
          Err(_) => Err(AdapterError::CdpTimeout(method.to_string())),
      }
  }
  ```

- [ ] 命令 ID 生成（原子递增）
  ```rust
  static COMMAND_ID: AtomicU64 = AtomicU64::new(1);
  
  fn next_command_id(&self) -> u64 {
      COMMAND_ID.fetch_add(1, Ordering::SeqCst)
  }
  ```

- [ ] 错误转换
  ```rust
  impl From<chromiumoxide::error::CdpError> for AdapterError {
      fn from(e: chromiumoxide::error::CdpError) -> Self {
          AdapterError::CdpIo(e.to_string())
      }
  }
  ```

**验收标准**:
- ✅ 命令成功发送和接收
- ✅ 超时正确触发
- ✅ 错误正确转换
- ✅ 并发命令互不干扰

---

### Week 3: 核心命令实现

#### Day 1-2: Navigate + Wait

**位置**: `crates/cdp-adapter/src/adapter.rs`

**任务清单**:
- [ ] **Navigate 实现**
  ```rust
  pub async fn navigate(
      &self,
      route: &ExecRoute,
      url: &str,
  ) -> Result<NavigateResult, AdapterError> {
      let page = self.get_page(route)?;
      
      // 发送 Page.navigate
      let response = self.transport.send_command(
          "Page.navigate",
          json!({ "url": url }),
          Duration::from_secs(30),
      ).await?;
      
      // 提取 frame_id
      let frame_id = response["frameId"].as_str()
          .ok_or(AdapterError::Internal("missing frameId"))?;
      
      Ok(NavigateResult {
          frame_id: frame_id.to_string(),
          loader_id: response["loaderId"].as_str().unwrap_or("").to_string(),
      })
  }
  ```

- [ ] **监听 Page.loadEventFired**
  ```rust
  // 在 handle_event() 中
  "Page.loadEventFired" => {
      let event = RawEvent::PageLoaded {
          page_id: self.resolve_page_id(&params)?,
          timestamp: Utc::now(),
      };
      self.bus.publish(event).await?;
  }
  ```

- [ ] **Wait 基础实现**
  ```rust
  pub async fn wait_for_navigation(
      &self,
      route: &ExecRoute,
      timeout: Duration,
  ) -> Result<(), AdapterError> {
      let page = self.get_page(route)?;
      
      // 等待 loadEventFired
      tokio::time::timeout(
          timeout,
          page.wait_for_navigation(),
      ).await??;
      
      Ok(())
  }
  ```

**验收标准**:
- ✅ 导航到 URL 成功
- ✅ loadEventFired 事件正确触发
- ✅ 超时正确处理
- ✅ 重定向正确跟踪

---

#### Day 3-4: Click + Type

**位置**: `crates/cdp-adapter/src/adapter.rs`

**任务清单**:
- [ ] **元素查询**
  ```rust
  async fn query_element(
      &self,
      route: &ExecRoute,
      selector: &str,
  ) -> Result<NodeId, AdapterError> {
      let response = self.transport.send_command(
          "DOM.querySelector",
          json!({
              "nodeId": self.get_document_node_id(route).await?,
              "selector": selector,
          }),
          Duration::from_secs(5),
      ).await?;
      
      let node_id = response["nodeId"].as_u64()
          .ok_or(AdapterError::ElementNotFound(selector.to_string()))?;
      
      Ok(NodeId(node_id))
  }
  ```

- [ ] **Click 实现**
  ```rust
  pub async fn click(
      &self,
      route: &ExecRoute,
      selector: &str,
  ) -> Result<(), AdapterError> {
      let node_id = self.query_element(route, selector).await?;
      
      // 获取元素位置
      let box_model = self.transport.send_command(
          "DOM.getBoxModel",
          json!({ "nodeId": node_id.0 }),
          Duration::from_secs(5),
      ).await?;
      
      let quad = &box_model["model"]["border"];
      let x = (quad[0].as_f64().unwrap() + quad[4].as_f64().unwrap()) / 2.0;
      let y = (quad[1].as_f64().unwrap() + quad[5].as_f64().unwrap()) / 2.0;
      
      // 滚动到可见
      self.scroll_into_view(route, node_id).await?;
      
      // 模拟鼠标点击
      self.dispatch_mouse_event(route, "mousePressed", x, y).await?;
      tokio::time::sleep(Duration::from_millis(50)).await;
      self.dispatch_mouse_event(route, "mouseReleased", x, y).await?;
      
      Ok(())
  }
  
  async fn dispatch_mouse_event(
      &self,
      route: &ExecRoute,
      event_type: &str,
      x: f64,
      y: f64,
  ) -> Result<(), AdapterError> {
      self.transport.send_command(
          "Input.dispatchMouseEvent",
          json!({
              "type": event_type,
              "x": x,
              "y": y,
              "button": "left",
              "clickCount": 1,
          }),
          Duration::from_secs(5),
      ).await?;
      Ok(())
  }
  ```

- [ ] **Type 实现**
  ```rust
  pub async fn type_text(
      &self,
      route: &ExecRoute,
      selector: &str,
      text: &str,
  ) -> Result<(), AdapterError> {
      let node_id = self.query_element(route, selector).await?;
      
      // Focus element
      self.transport.send_command(
          "DOM.focus",
          json!({ "nodeId": node_id.0 }),
          Duration::from_secs(5),
      ).await?;
      
      // Clear existing content (Ctrl+A, Delete)
      self.dispatch_key_event("keyDown", "Control").await?;
      self.dispatch_key_event("char", "a").await?;
      self.dispatch_key_event("keyUp", "Control").await?;
      self.dispatch_key_event("keyDown", "Delete").await?;
      self.dispatch_key_event("keyUp", "Delete").await?;
      
      // Type each character
      for ch in text.chars() {
          self.dispatch_key_event("char", &ch.to_string()).await?;
          tokio::time::sleep(Duration::from_millis(20)).await; // 人类化节奏
      }
      
      Ok(())
  }
  
  async fn dispatch_key_event(
      &self,
      event_type: &str,
      key: &str,
  ) -> Result<(), AdapterError> {
      self.transport.send_command(
          "Input.dispatchKeyEvent",
          json!({
              "type": event_type,
              "text": key,
          }),
          Duration::from_secs(5),
      ).await?;
      Ok(())
  }
  ```

**验收标准**:
- ✅ 元素成功查询
- ✅ 点击正确触发
- ✅ 文本正确输入
- ✅ 人类化节奏生效

---

#### Day 5-6: Select + Screenshot + Snapshot

**位置**: `crates/cdp-adapter/src/adapter.rs`

**任务清单**:
- [ ] **Select 实现**
  ```rust
  pub async fn select(
      &self,
      route: &ExecRoute,
      selector: &str,
      value: &str,
  ) -> Result<(), AdapterError> {
      let script = format!(
          r#"
          (function(selector, value) {{
              const select = document.querySelector(selector);
              if (!select) return false;
              select.value = value;
              select.dispatchEvent(new Event('change', {{ bubbles: true }}));
              return true;
          }})('{}', '{}')
          "#,
          selector, value
      );
      
      let result = self.evaluate_script(route, &script).await?;
      
      if !result.as_bool().unwrap_or(false) {
          return Err(AdapterError::ElementNotFound(selector.to_string()));
      }
      
      Ok(())
  }
  ```

- [ ] **Screenshot 实现**
  ```rust
  pub async fn capture_screenshot(
      &self,
      route: &ExecRoute,
      options: ScreenshotOptions,
  ) -> Result<Screenshot, AdapterError> {
      let response = self.transport.send_command(
          "Page.captureScreenshot",
          json!({
              "format": options.format,  // "png" or "jpeg"
              "quality": options.quality, // 0-100
              "clip": options.clip,       // 可选裁剪区域
          }),
          Duration::from_secs(10),
      ).await?;
      
      let data = response["data"].as_str()
          .ok_or(AdapterError::Internal("missing screenshot data"))?;
      
      Ok(Screenshot {
          data: data.to_string(),  // Base64
          format: options.format,
          timestamp: Utc::now(),
      })
  }
  ```

- [ ] **DOM Snapshot 实现**
  ```rust
  pub async fn capture_dom_snapshot(
      &self,
      route: &ExecRoute,
  ) -> Result<DomSnapshot, AdapterError> {
      let response = self.transport.send_command(
          "DOMSnapshot.captureSnapshot",
          json!({
              "computedStyles": ["width", "height", "display", "visibility"],
          }),
          Duration::from_secs(15),
      ).await?;
      
      Ok(DomSnapshot {
          documents: serde_json::from_value(response["documents"].clone())?,
          strings: serde_json::from_value(response["strings"].clone())?,
      })
  }
  ```

**验收标准**:
- ✅ Select 正确设置值
- ✅ Screenshot 正确捕获
- ✅ DOM Snapshot 完整

---

#### Day 7: 单元测试与本地验证

**位置**: `crates/cdp-adapter/tests/`

**任务清单**:
- [ ] **连接测试**
  ```rust
  #[tokio::test]
  async fn test_browser_launch() {
      let transport = ChromiumTransport::new().await.unwrap();
      assert!(transport.is_connected());
  }
  ```

- [ ] **命令测试**
  ```rust
  #[tokio::test]
  async fn test_navigate() {
      let adapter = CdpAdapter::new().await.unwrap();
      let route = ExecRoute::default();
      
      adapter.navigate(&route, "https://example.com").await.unwrap();
      
      let url = adapter.get_current_url(&route).await.unwrap();
      assert!(url.contains("example.com"));
  }
  ```

- [ ] **事件测试**
  ```rust
  #[tokio::test]
  async fn test_page_load_event() {
      let adapter = CdpAdapter::new().await.unwrap();
      let mut events = adapter.subscribe_events().await;
      
      let route = ExecRoute::default();
      adapter.navigate(&route, "https://example.com").await.unwrap();
      
      let event = events.recv().await.unwrap();
      assert!(matches!(event, RawEvent::PageLoaded { .. }));
  }
  ```

**运行测试**:
```bash
# 设置环境变量
export SOULBROWSER_USE_REAL_CHROME=1
export SOULBROWSER_CHROME=/usr/bin/google-chrome

# 运行测试
cargo test -p cdp-adapter
```

**验收标准**:
- ✅ 所有单元测试通过
- ✅ 本地浏览器成功启动
- ✅ 核心命令验证通过

---

### Week 4: 自愈与集成

#### Day 1-2: 断线重连

**位置**: `crates/cdp-adapter/src/transport.rs`

**任务清单**:
- [ ] **连接健康检查**
  ```rust
  pub async fn check_health(&self) -> bool {
      match self.transport.send_command(
          "Browser.getVersion",
          json!({}),
          Duration::from_secs(3),
      ).await {
          Ok(_) => true,
          Err(_) => false,
      }
  }
  ```

- [ ] **重连逻辑**
  ```rust
  pub async fn reconnect(&mut self) -> Result<(), AdapterError> {
      tracing::warn!("Connection lost, attempting reconnect...");
      
      // 关闭旧连接
      self.close().await?;
      
      // 重新启动浏览器
      let (browser, handler) = Browser::launch(self.config.clone()).await?;
      self.browser = browser;
      self.handler = handler;
      
      // 重新启动事件循环
      self.start_event_loop();
      
      tracing::info!("Reconnected successfully");
      Ok(())
  }
  ```

- [ ] **自动重连机制**
  ```rust
  async fn event_loop(&mut self) {
      loop {
          tokio::select! {
              Some(event) = self.handler.next() => {
                  if let Err(e) = self.handle_event(event).await {
                      tracing::error!("Event handling error: {}", e);
                      
                      // 检测是否需要重连
                      if e.is_connection_error() {
                          if let Err(e) = self.reconnect().await {
                              tracing::error!("Reconnect failed: {}", e);
                              break;
                          }
                      }
                  }
              }
              else => break,
          }
      }
  }
  ```

**验收标准**:
- ✅ 检测到断线
- ✅ 自动重连成功
- ✅ 状态正确恢复
- ✅ 重连失败正确报错

---

#### Day 3-4: 事件解析

**位置**: `crates/cdp-adapter/src/adapter.rs`

**任务清单**:
- [ ] **TransportEvent → RawEvent 映射**
  ```rust
  async fn handle_event(&self, event: TransportEvent) -> Result<()> {
      let raw_event = match event.method.as_str() {
          "Page.loadEventFired" => {
              RawEvent::PageLoaded {
                  page_id: self.resolve_page_id(&event.params)?,
                  timestamp: Utc::now(),
              }
          }
          "Page.frameAttached" => {
              RawEvent::FrameAttached {
                  frame_id: event.params["frameId"].as_str().unwrap().to_string(),
                  parent_id: event.params["parentFrameId"].as_str().unwrap().to_string(),
              }
          }
          "Network.requestWillBeSent" => {
              RawEvent::NetworkRequest {
                  request_id: event.params["requestId"].as_str().unwrap().to_string(),
                  url: event.params["request"]["url"].as_str().unwrap().to_string(),
                  method: event.params["request"]["method"].as_str().unwrap().to_string(),
              }
          }
          "Network.responseReceived" => {
              RawEvent::NetworkResponse {
                  request_id: event.params["requestId"].as_str().unwrap().to_string(),
                  status: event.params["response"]["status"].as_u64().unwrap() as u16,
              }
          }
          "Runtime.exceptionThrown" => {
              RawEvent::JSException {
                  message: event.params["exceptionDetails"]["text"].as_str().unwrap().to_string(),
              }
          }
          _ => return Ok(()), // 忽略未处理的事件
      };
      
      // 发布到 EventBus
      self.bus.publish(raw_event).await?;
      
      Ok(())
  }
  ```

- [ ] **Registry 更新**
  ```rust
  // 在 Registry 中订阅事件
  let mut events = adapter.subscribe_events().await;
  
  tokio::spawn(async move {
      while let Some(event) = events.recv().await {
          match event {
              RawEvent::PageLoaded { page_id, .. } => {
                  registry.mark_page_loaded(page_id).await;
              }
              RawEvent::FrameAttached { frame_id, parent_id } => {
                  registry.attach_frame(parent_id, frame_id).await;
              }
              _ => {}
          }
      }
  });
  ```

**验收标准**:
- ✅ 所有关键事件正确解析
- ✅ Registry 状态正确更新
- ✅ 事件总线正确分发

---

#### Day 5-7: 集成测试

**位置**: `tests/l0_cdp_integration.rs`

**任务清单**:
- [ ] **端到端测试**
  ```rust
  #[tokio::test]
  async fn test_full_navigation_flow() {
      let adapter = CdpAdapter::new().await.unwrap();
      let registry = Registry::new();
      let bus = EventBus::new();
      
      // 订阅事件
      let mut events = bus.subscribe("page").await;
      
      // 创建 session 和 page
      let session_id = registry.create_session().await;
      let page_id = registry.create_page(session_id).await;
      let route = ExecRoute { session_id, page_id, frame_id: None };
      
      // 导航
      adapter.navigate(&route, "https://example.com").await.unwrap();
      
      // 等待 PageLoaded 事件
      let event = tokio::time::timeout(
          Duration::from_secs(10),
          events.recv(),
      ).await.unwrap().unwrap();
      
      assert!(matches!(event, RawEvent::PageLoaded { .. }));
      
      // 验证 URL
      let url = adapter.get_current_url(&route).await.unwrap();
      assert!(url.contains("example.com"));
  }
  ```

- [ ] **性能测试**
  ```rust
  #[tokio::test]
  async fn test_command_performance() {
      let adapter = CdpAdapter::new().await.unwrap();
      let route = ExecRoute::default();
      
      adapter.navigate(&route, "https://example.com").await.unwrap();
      
      // 测试 100 次点击性能
      let start = Instant::now();
      for _ in 0..100 {
          adapter.click(&route, "#link").await.unwrap();
      }
      let elapsed = start.elapsed();
      
      let avg_ms = elapsed.as_millis() / 100;
      assert!(avg_ms < 100, "Average click time: {}ms", avg_ms);
  }
  ```

- [ ] **故障注入测试**
  ```rust
  #[tokio::test]
  async fn test_reconnect_on_disconnect() {
      let mut adapter = CdpAdapter::new().await.unwrap();
      
      // 模拟断线（关闭浏览器）
      adapter.close_browser().await.unwrap();
      
      // 发送命令应触发重连
      let result = adapter.navigate(&route, "https://example.com").await;
      
      // 重连后命令应成功
      assert!(result.is_ok());
  }
  ```

**验收标准**:
- ✅ E2E 测试全部通过
- ✅ 性能达标（P95 < 500ms）
- ✅ 故障恢复正常

---

## 🎯 Milestone 2: L0 卫星模块（4-5 周）

### Week 5-6: Permissions Broker（2 周）

**位置**: `crates/permissions-broker/`

**任务清单**:

#### Week 5: 策略解析与缓存

- [ ] **Day 1-2: 策略模板解析**
  ```rust
  pub struct PermissionPolicy {
      pub origin: String,
      pub permission: String,
      pub decision: Decision,
      pub ttl: Duration,
  }
  
  pub fn load_policies(path: &Path) -> Result<Vec<PermissionPolicy>> {
      let content = std::fs::read_to_string(path)?;
      let policies: Vec<PermissionPolicy> = serde_yaml::from_str(&content)?;
      Ok(policies)
  }
  ```

- [ ] **Day 3-4: Per-origin 缓存**
  ```rust
  pub struct PermissionCache {
      cache: Arc<DashMap<String, CachedDecision>>,
  }
  
  pub struct CachedDecision {
      pub decision: Decision,
      pub expires_at: Instant,
  }
  
  impl PermissionCache {
      pub fn get(&self, origin: &str, permission: &str) -> Option<Decision> {
          let key = format!("{}:{}", origin, permission);
          self.cache.get(&key).and_then(|entry| {
              if entry.expires_at > Instant::now() {
                  Some(entry.decision.clone())
              } else {
                  None
              }
          })
      }
      
      pub fn insert(&self, origin: &str, permission: &str, decision: Decision, ttl: Duration) {
          let key = format!("{}:{}", origin, permission);
          self.cache.insert(key, CachedDecision {
              decision,
              expires_at: Instant::now() + ttl,
          });
      }
  }
  ```

- [ ] **Day 5: 单元测试**

#### Week 6: CDP 集成与审计

- [ ] **Day 1-2: CDP Permissions API 集成**
  ```rust
  pub async fn set_permission_override(
      &self,
      origin: &str,
      permission: &str,
      decision: Decision,
  ) -> Result<()> {
      let state = match decision {
          Decision::Allow => "granted",
          Decision::Deny => "denied",
          Decision::Prompt => "prompt",
      };
      
      self.adapter.send_command(
          "Browser.setPermission",
          json!({
              "origin": origin,
              "permission": { "name": permission },
              "setting": state,
          }),
          Duration::from_secs(5),
      ).await?;
      
      Ok(())
  }
  ```

- [ ] **Day 3-4: 审计事件发布**
  ```rust
  pub async fn check_permission(
      &self,
      origin: &str,
      permission: &str,
  ) -> Result<Decision> {
      // 检查缓存
      if let Some(decision) = self.cache.get(origin, permission) {
          return Ok(decision);
      }
      
      // 查询策略
      let decision = self.policies.get(origin, permission)
          .unwrap_or(Decision::Prompt);
      
      // 发布审计事件
      self.bus.publish(RawEvent::PermissionCheck {
          origin: origin.to_string(),
          permission: permission.to_string(),
          decision: decision.clone(),
          timestamp: Utc::now(),
      }).await?;
      
      // 缓存结果
      self.cache.insert(origin, permission, decision.clone(), self.default_ttl);
      
      Ok(decision)
  }
  ```

- [ ] **Day 5: 集成测试**

**验收标准**:
- ✅ 策略正确加载
- ✅ 缓存正确失效
- ✅ CDP 权限正确设置
- ✅ 审计事件正确发布

---

### Week 7: Network Tap Light（1.5 周）

**位置**: `crates/network-tap-light/`

**任务清单**:

- [ ] **Day 1-2: 事件聚合**
  ```rust
  pub struct NetworkTap {
      state: Arc<Mutex<TapState>>,
  }
  
  pub struct TapState {
      inflight: HashMap<String, RequestInfo>,
      summary: NetworkSummary,
      last_activity: Instant,
  }
  
  impl NetworkTap {
      pub async fn handle_request(&self, request: NetworkRequest) {
          let mut state = self.state.lock().await;
          state.inflight.insert(request.id.clone(), RequestInfo {
              url: request.url,
              method: request.method,
              started_at: Instant::now(),
          });
          state.last_activity = Instant::now();
      }
      
      pub async fn handle_response(&self, response: NetworkResponse) {
          let mut state = self.state.lock().await;
          if let Some(req) = state.inflight.remove(&response.request_id) {
              // 更新统计
              match response.status {
                  200..=299 => state.summary.count_2xx += 1,
                  400..=499 => state.summary.count_4xx += 1,
                  500..=599 => state.summary.count_5xx += 1,
                  _ => {}
              }
          }
          state.last_activity = Instant::now();
      }
  }
  ```

- [ ] **Day 3-4: 安静检测**
  ```rust
  pub async fn is_quiet(&self, threshold_ms: u64) -> bool {
      let state = self.state.lock().await;
      state.inflight.is_empty() &&
          state.last_activity.elapsed().as_millis() >= threshold_ms as u128
  }
  
  pub async fn wait_for_quiet(&self, threshold_ms: u64, timeout: Duration) -> Result<()> {
      let start = Instant::now();
      loop {
          if self.is_quiet(threshold_ms).await {
              return Ok(());
          }
          if start.elapsed() > timeout {
              return Err(AdapterError::WaitTimeout("network quiet".to_string()));
          }
          tokio::time::sleep(Duration::from_millis(100)).await;
      }
  }
  ```

- [ ] **Day 5: NetworkSummary 发布**
  ```rust
  pub async fn publish_summary(&self) {
      let state = self.state.lock().await;
      let summary = state.summary.clone();
      
      self.bus.publish(RawEvent::NetworkSummary {
          page_id: state.page_id,
          summary,
      }).await.ok();
  }
  ```

**验收标准**:
- ✅ 网络事件正确聚合
- ✅ 安静检测准确
- ✅ 摘要正确发布

---

### Week 8-9: Stealth + Extensions Bridge（2 周）

#### Week 8: Stealth 功能

**位置**: `crates/stealth/`

**任务清单**:
- [ ] **Day 1-2: Profile 解析**
  ```rust
  pub struct StealthProfile {
      pub user_agent: String,
      pub viewport: Viewport,
      pub timezone: String,
      pub locale: String,
      pub webgl_vendor: Option<String>,
  }
  
  pub fn load_profile(name: &str) -> Result<StealthProfile> {
      let path = format!("config/stealth/{}.yaml", name);
      let content = std::fs::read_to_string(&path)?;
      let profile: StealthProfile = serde_yaml::from_str(&content)?;
      Ok(profile)
  }
  ```

- [ ] **Day 3-4: CDP 注入**
  ```rust
  pub async fn apply_profile(
      &self,
      adapter: &CdpAdapter,
      profile: &StealthProfile,
  ) -> Result<()> {
      // User Agent
      adapter.send_command(
          "Emulation.setUserAgentOverride",
          json!({ "userAgent": profile.user_agent }),
          Duration::from_secs(5),
      ).await?;
      
      // Viewport
      adapter.send_command(
          "Emulation.setDeviceMetricsOverride",
          json!({
              "width": profile.viewport.width,
              "height": profile.viewport.height,
              "deviceScaleFactor": 1,
              "mobile": false,
          }),
          Duration::from_secs(5),
      ).await?;
      
      // Timezone
      adapter.send_command(
          "Emulation.setTimezoneOverride",
          json!({ "timezoneId": profile.timezone }),
          Duration::from_secs(5),
      ).await?;
      
      Ok(())
  }
  ```

- [ ] **Day 5: 测试**

#### Week 9: Extensions Bridge

**位置**: `crates/extensions-bridge/`

**任务清单**:
- [ ] **Day 1-2: 白名单加载**
- [ ] **Day 3-4: 通道握手**
- [ ] **Day 5: 测试**

---

## 🎯 Milestone 3: CLI 集成与验收（2-3 周）

### Week 10-11: 集成与测试

**任务清单**:
- [ ] Feature flags 推出
- [ ] 配置接线
- [ ] E2E 测试
- [ ] 可观测性扩展
- [ ] 文档更新

### Week 12: 验收

**任务清单**:
- [ ] 功能验收
- [ ] 性能验收
- [ ] 故障注入验收
- [ ] 安全审查

---

## 📊 验收标准

### 功能验收
- ✅ 8 个核心命令全部可用
- ✅ 所有模块集成正常
- ✅ CLI 命令全部工作

### 性能验收
- ✅ 连接稳定性 > 99%
- ✅ 命令执行 P95 < 500ms
- ✅ 重连成功率 100%

### 质量验收
- ✅ 单元测试覆盖率 > 80%
- ✅ 集成测试全部通过
- ✅ 无内存泄漏
- ✅ 无 Clippy 警告

---

**文档维护**: 每个 Milestone 完成后更新进度。
