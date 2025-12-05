# SoulBrowser Web Console 使用指南

本指南介绍如何使用 SoulBrowser 的现代化 Web 控制台界面。

## 快速开始

### 1. 启动后端服务

首先启动 SoulBrowser 后端服务器：

```bash
cd /mnt/d/github/SoulBrowserClaude/SoulBrowser

# 方式 1: 直接启动（如果已编译）
./target/release/soulbrowser --metrics-port 0 serve --port 8787

# 方式 2: 通过 cargo 运行
cargo run --bin soulbrowser -- --metrics-port 0 serve --port 8787
```

服务器将在 `http://localhost:8787` 启动，并默认启用鉴权（Serve/API 优化计划要求）。

- **访问令牌**：启动日志里会打印 `Generated serve auth token: <TOKEN>`（或使用 `--auth-token/--allow-ip` 显式设置）。前端和脚本调用时需要通过 `x-soulbrowser-token` 头或 `Authorization: Bearer <TOKEN>` 传入。
- **允许的 IP**：默认只允许 `127.0.0.1/::1`，如需在局域网调试可以追加 `--allow-ip 192.168.1.10` 等参数。
- **取消鉴权（仅限本机调试）**：必须显式传入 `--disable-auth`，否则 Serve 会拒绝未授权的访问。

### 2. 启动前端界面

在新的终端窗口中启动前端开发服务器：

```bash
cd web-console
npm install  # 首次运行需要安装依赖
npm run dev
```

前端将在 `http://localhost:5173` 启动。

### 3. 打开浏览器

访问 `http://localhost:5173`，你将看到 SoulBrowser Web Console 界面。

---

## 功能介绍

### 🔍 感知测试（Perceive）

**位置**: 首页 `/perceive`

**功能**: 执行多模态页面感知，分析网页的结构、视觉和语义信息。

**使用步骤**:

1. 输入目标 URL（例如：`https://example.com`）
2. 选择感知模式：
   - **All** - 完整分析（结构 + 视觉 + 语义）
   - **Structural Only** - 仅分析 DOM 结构
   - **Visual Only** - 仅分析视觉特征
   - **Semantic Only** - 仅分析语义内容
   - **Custom Selection** - 自定义选择分析维度
3. 可选项：
   - ✅ **Capture Screenshot** - 捕获页面截图
   - ✅ **Generate Insights** - 生成智能洞察
4. 点击 **Run Perception** 开始分析

**结果展示**:

- **🏗️ Structural Perception** - DOM 节点数、表单数、交互元素数
- **🎨 Visual Perception** - 视口尺寸、主色调
- **🧠 Semantic Perception** - 内容类型、主标题、语言
- **💡 Insights** - 智能分析建议
- **📸 Screenshot** - 页面截图（如果启用）
- **📝 Raw JSON** - 完整的感知数据 JSON

**示例**:

```
URL: https://github.com
Mode: All
Screenshot: ✅
Insights: ✅
```

结果将显示 GitHub 首页的完整分析，包括 DOM 结构统计、主色调、语义信息和截图。

### 💬 AI 对话（Chat）

**位置**: `/chat`

**功能**: 使用 L8 Agent 接口生成自动化任务计划。

**使用步骤**:

1. 左侧可以选择预定义的任务模板，或点击"自定义任务"
2. 在输入框中描述你想要的自动化任务
3. 点击发送按钮或按 Enter 键
4. AI 将生成详细的任务执行计划

**任务模板**:

- **📝 表单自动填写** - 自动填写网页表单
- **🔍 网页数据采集** - 从网页提取数据
- **🧪 自动化测试** - 执行自动化测试流程
- **📊 竞品监控** - 监控竞品价格变化

**任务计划展示**:

生成的计划将包含：
- 预计耗时
- 风险等级
- 成功率预估
- 详细的执行步骤
- 策略检查结果

**示例对话**:

```
用户: 帮我登录 example.com 并填写联系表单

AI: 好的，我已经为你生成了任务计划，包括以下步骤：
1. 导航到 example.com
2. 定位并填写用户名
3. 定位并填写密码
4. 点击登录按钮
5. 导航到联系表单页面
...
```

### 📋 任务管理（Tasks）

**位置**: `/tasks`

**功能**: 管理和监控所有自动化任务的执行状态。前端现在使用新的 SSE `/api/tasks/:task_id/events`，支持 `Last-Event-ID` 自动重连，断网时也能补齐历史事件。

**功能**:

- 查看所有任务列表
- 实时任务状态更新（SSE 持续推送 agent 历史、Watchdog 事件、警报等）
- 任务进度条显示
- 任务操作：
  - ▶️ **开始** - 启动待执行的任务
  - ⏸️ **暂停** - 暂停运行中的任务
  - ❌ **取消** - 取消任务执行
  - 🔄 **重试** - 重新执行失败的任务

**筛选功能**:

- 搜索框：按任务名称搜索
- 状态筛选：运行中、已完成、失败

**API 端点补充**:

- `GET /api/tasks/:id/logs?limit=100&cursor=<last_cursor>&since=2025-01-01T00:00:00Z` —— 日志分页接口，返回 `next_cursor` 供增量拉取。
- `GET /api/tasks/:id/events` —— SSE 事件流，支持 `Last-Event-ID` 头自动补齐丢失事件。
- `GET /api/tasks/:id/stream` —— WebSocket 兼容接口，保留给 legacy 前端。

### 📊 监控仪表盘（Dashboard）

**位置**: `/dashboard`

**功能**: 实时监控系统性能和任务统计。

**展示内容**:

- **统计卡片**:
  - 总任务数
  - 成功率
  - 平均耗时
  - 失败任务数

- **趋势图表**:
  - 任务成功率趋势（折线图）
  - 任务数量统计（柱状图）

- **实时更新**: 每 30 秒自动刷新数据

### 🛠️ 诊断与治理（Diagnostics）

**位置**: `/diagnostics`

**功能**: 聚合 Memory 统计、自愈策略、插件注册表、录制会话以及 `/health`/`/api/chat`/`/api/perceive` 快速检测，帮助 Oncall 快速定位问题。

- **Memory 统计**: 显示命中率、模板使用情况与累计写入/删除，命中率低于 50% 会高亮告警。
- **自愈策略**: 列出 `config/self_heal.yaml` 中的策略，可直接切换启用状态（调用 `POST /api/self-heal/strategies/:id`）。
- **插件注册表**: 新增卡片展示 `active/pending/disabled` 数量、最近审阅时间，可以通过下拉框切换单个插件状态，等价于 CLI/REST；当条目定义 `helpers[]`（pattern + 多步骤 DSL）时，这里会列出所有 helper 及其阻塞标签，并提供“新增/编辑/删除”表单（字段与 `RegistryHelper` DSL 对齐），成功后立即调用 `/api/plugins/registry/:id/helpers` CRUD 接口。
- **录制会话/健康检查**: 直接回放录制详情、重新跑 Chat/Perceive 测试请求，定位执行层面的异常。

---

## 常见问题

### Q1: 无法连接到后端服务

**症状**: 前端显示"服务暂时不可用"

**解决方案**:

1. 确认后端服务已启动
2. 检查端口是否正确（默认 8787）
3. 查看后端日志是否有错误

```bash
# 检查服务是否运行
curl http://localhost:8787/health
```

### Q2: Perceive 功能报错

**症状**: "multi-modal perception failed"

**可能原因**:

1. **Chrome 无法启动** - WSL 环境下没有配置外部 Chrome
2. **权限问题** - Chrome 启动需要沙箱权限

**解决方案**:

**方式 1: 连接到 Windows Chrome（推荐用于 WSL）**

```bash
# 在 Windows PowerShell 中启动 Chrome
"C:\Program Files\Google\Chrome\Application\chrome.exe" --remote-debugging-port=9222 --user-data-dir=C:\ChromeRemote

# 在 WSL 中设置环境变量
export SOULBROWSER_WS_URL=http://127.0.0.1:9222

# 启动服务
cargo run --bin soulbrowser -- --metrics-port 0 serve --port 8787 --ws-url http://127.0.0.1:9222
```

**方式 2: 禁用沙箱**

```bash
export SOULBROWSER_DISABLE_SANDBOX=1
cargo run --bin soulbrowser -- --metrics-port 0 serve --port 8787
```

### Q3: Chat 功能无响应

**症状**: 发送消息后没有回复

**可能原因**: Chat API 端点未实现或后端错误

**解决方案**:

1. 查看后端日志
2. 确认 `/api/chat` 端点已实现
3. 尝试使用 CLI 命令测试：

```bash
cargo run --bin soulbrowser -- chat --prompt "帮我登录 example.com"
```

### Q4: 前端编译错误

**症状**: `npm run dev` 报错

**解决方案**:

```bash
# 清理并重新安装依赖
rm -rf node_modules package-lock.json
npm install

# 如果仍然有问题，尝试使用特定 Node 版本
nvm install 18
nvm use 18
npm install
```

---

## 高级配置

### 自定义端口

**后端端口**:

```bash
cargo run --bin soulbrowser -- serve --port 9000
```

然后修改 `web-console/vite.config.ts`:

```typescript
proxy: {
  '/api': 'http://localhost:9000',
}
```

**前端端口**:

修改 `web-console/vite.config.ts`:

```typescript
server: {
  port: 3000,
}
```

### 存储保洁与并发控制

- `SOUL_PLAN_TTL_DAYS`（默认 30）：Serve 启动时会扫描 `soulbrowser-output/tasks/*.json` 并清理早于该 TTL 的计划文件；设置为 `0` 可禁用自动清理。
- `SOUL_CHAT_CONTEXT_LIMIT`（默认 2）：限制 `/api/chat` 触发的感知上下文抓取同时运行数量，防止感知任务拖垮整个 Web Console。

### 生产部署

#### 构建前端

```bash
cd web-console
npm run build
```

构建产物位于 `web-console/dist/`。

#### 使用 Nginx 反向代理

```nginx
server {
  listen 80;
  server_name soulbrowser.example.com;

  # 前端静态文件
  location / {
    root /path/to/web-console/dist;
    try_files $uri /index.html;
  }

  # 后端 API 代理
  location /api/ {
    proxy_pass http://localhost:8787;
    proxy_set_header Host $host;
    proxy_set_header X-Real-IP $remote_addr;
  }

  # 健康检查
  location /health {
    proxy_pass http://localhost:8787;
  }
}
```

---

## 开发技巧

### 1. 使用浏览器开发者工具

- **F12** 打开开发者工具
- **Network** 标签查看 API 请求
- **Console** 标签查看日志和错误
- **React DevTools** 调试组件状态

### 2. 热更新

Vite 提供了快速的 HMR，修改代码后页面会自动刷新。

### 3. 调试后端

查看后端日志：

```bash
# 启用详细日志
RUST_LOG=debug cargo run --bin soulbrowser -- serve --port 8787
```

### 4. API 测试

使用 curl 或 Postman 测试 API：

```bash
# 测试 perceive API
curl -X POST http://localhost:8787/api/perceive \
  -H "Content-Type: application/json" \
  -d '{
    "url": "https://example.com",
    "mode": "all",
    "screenshot": true,
    "insights": true
  }'

# 健康检查
curl http://localhost:8787/health
```

可选字段：

- `viewport`：`{"width":1280,"height":720,"device_scale_factor":1.0,"mobile":false,"emulate_touch":false}`，执行前覆写 Chrome 视口尺寸 / 触控行为；
- `cookies`：`[{"name":"session","value":"abc","domain":".example.com","same_site":"lax"}]` 将在导航前通过 CDP 注入 Cookie，适合复现登录态；
- `inject_script`：传入一段 JS 字符串（如 `"(() => window.initTest && window.initTest())()"`），会在 DOM Ready 后执行并将返回值写入后台日志，方便打桩或预热页面。

---

## 下一步

- 查看 [FRONTEND_SETUP_GUIDE.md](./FRONTEND_SETUP_GUIDE.md) 了解前端开发环境配置
- 查看 [VISUAL_TESTING_CONSOLE.md](./VISUAL_TESTING_CONSOLE.md) 了解原始控制台的详细信息
- 查看 [UI_DEVELOPMENT_PLAN.md](./UI_DEVELOPMENT_PLAN.md) 了解未来功能规划

---

**享受使用 SoulBrowser！** 🚀
