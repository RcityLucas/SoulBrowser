# L8 Visual & Perception Testing Guide

> 目标：复用 RainbowBrowserAI 的可视化测试经验，在 SoulBrowser 中建立既可自动化验证又可人工核对的多模态感知测试流程，为后续对话式 Agent 的视觉锚点与高亮体验打下基线。

## 测试结构概览

| 层次 | 目的 | 对应工具 |
|------|------|----------|
| 自动化校验 | 验证 `soulbrowser perceive` 等 CLI 输出结构、置信度与截图产物 | `tests/l8_visual_suite.sh` |
| 手动可视化 | 观察 CLI 输出、截图与（未来的）控制台高亮效果 | 终端 + 截图查看器（后续扩展到 Web 控制台） |
| 数据回放 | 通过 `soulbrowser perceiver` 检查事件摘要，为 UI 叠层与回放做准备 | `soulbrowser perceiver --format json` |

## 前置条件

1. 安装真实的 Chrome/Chromium。脚本会尝试自动检测常见路径（`google-chrome-stable`, `chromium`, `/Applications/Google Chrome.app/...`, `/mnt/c/Program Files/.../chrome.exe` 等），并在 WSL 环境自动启用 `SOULBROWSER_DISABLE_SANDBOX=1`。若仍未找到或仍需自定义，再手动配置环境变量：
   ```bash
   export SOULBROWSER_USE_REAL_CHROME=1
   export SOULBROWSER_CHROME="/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"
   ```
2. 若在 WSL 内无法直接启动 Linux Chrome，可在 Windows 主机打开命令行执行：
   ```powershell
   "C:\Program Files\Google\Chrome\Application\chrome.exe" --remote-debugging-port=9222 --user-data-dir=C:\ChromeRemote
   ```
   然后在 WSL 中设置 `SOULBROWSER_WS_URL=http://127.0.0.1:9222` 后运行脚本，脚本会直接连接该 DevTools 端口而不再尝试启动本地浏览器。
3. 准备 `jq`（解析 JSON）与 `python`（成功率计算，随 Python3 自带）。
4. 在 `SoulBrowser/` 目录运行 `cargo build --release`，确保依赖已编译。

## 自动化测试脚本

位置：`tests/l8_visual_suite.sh`

运行：
```bash
cd SoulBrowser
./tests/l8_visual_suite.sh
```

脚本完成的检查与 RainbowBrowserAI 流程保持一致：

1. **结构化感知**：调用 `soulbrowser perceive --structural`，输出 JSON 至临时文件，验证 `structural.dom_node_count`、`confidence`。
2. **视觉感知 + 截图**：调用 `--visual --screenshot tmp.png`，校验 `visual.screenshot_id`、`visual.avg_contrast` 并确保截图非空。
3. **全模态 + 洞察**：运行 `--all --insights`，检查结构/语义/洞察字段齐全，为未来对话计划生成提供依据。
4. **感知事件摘要**：调用 `soulbrowser perceiver --format json`，确认 `resolve` 等统计，为控制台回放与高亮叠层准备数据面。

所有步骤以 ✅ / ❌ 显示，最终输出成功率百分比。若 State Center 尚无事件，会给出告警信息但不判定失败。

### 环境变量

- `TEST_URL`：自定义测试 URL（默认 `https://example.com`）。
- `PERCEPTION_TIMEOUT`：感知超时时间，默认 45 秒。

## 手动可视化核对

1. **结构化输出**：
   ```bash
   cargo run -- perceive --url https://example.com --structural --output structural.json
   jq . structural.json
   ```
   核对 DOM 数量、交互元素统计与快照 ID。

2. **视觉输出与截图**：
   ```bash
   cargo run -- perceive --url https://example.com --visual --screenshot visual.png --output visual.json
   open visual.png   # macOS；Linux 使用 xdg-open，Windows 使用 start
   jq '.visual' visual.json
   ```
   确认 dominant colors、对比度、视口占比等数据与截图一致。

3. **全模态+洞察**：
   ```bash
   cargo run -- perceive --url https://example.com --all --insights --output full.json
   jq '.semantic, .insights' full.json
   ```
   校验语义摘要、语言、洞察类型及置信度，记录潜在问题。

4. **事件回放数据**：
   ```bash
   soulbrowser perceiver --format table --limit 5
   ```
   观察 resolve / judge / snapshot 事件，确认缓存命中率等指标是否符合预期。

5. **（预留）控制台高亮验证**：阶段 1 实现 Web 控制台后，按照以下步骤补充：
   - 在控制台对话中执行同样的感知操作。
   - 使用“高亮”开关确认图像层与 DOM 锚点一致。
   - 记录高亮区域坐标，并与 `visual_anchor` JSON 对照。

## Web 可视化控制台（实验版）

为了免去命令行操作，你可以启动内置的轻量测试服务器并通过浏览器操作感知任务：

```bash
cd SoulBrowser
SOULBROWSER_USE_REAL_CHROME=1 \
SOULBROWSER_CHROME=/path/to/chrome \
soulbrowser --metrics-port 0 serve --port 8787
```

> 若在无浏览器环境（CI、本地离线调试）需要验证控制台接口，可设置 `SOULBROWSER_CONSOLE_FIXTURE=/path/to/fixture.json` 来启用模拟输出；必要时再通过 `SOULBROWSER_CONSOLE_FIXTURE_SCREENSHOT` 指定截图 PNG 文件。

- 如果你在 WSL 或容器内，Chrome 可能无法直接启动。可在宿主机执行：
  ```powershell
  "C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe" --remote-debugging-port=9222 --user-data-dir=C:\\ChromeRemote
  ```
  然后在 WSL 中以 `SOULBROWSER_WS_URL=http://127.0.0.1:9222 soulbrowser --metrics-port 0 serve …` 方式启动，测试服务器会连接到这个 DevTools 端口。
- 打开浏览器访问 `http://localhost:8787`，输入 URL 并选择感知模式，点击 “Run Perception” 即可实时查看结构/视觉/语义输出、stdout/stderr 以及截图。

测试服务器的 `/api/perceive` 接口在内部调用 `soulbrowser perceive` 子命令，因此仍依赖真实 Chrome。如果返回 `multi-modal perception failed`，可参考本指南前面的排障步骤或检查外部 Chrome 是否已启动。

## 常见问题与排查

| 现象 | 可能原因 | 处理方法 |
|------|----------|----------|
| CLI 提示未启用真实 Chrome | `SOULBROWSER_USE_REAL_CHROME` 未设置 | `export SOULBROWSER_USE_REAL_CHROME=1` |
| 感知超时 | 网页复杂或网络慢 | 调整 `PERCEPTION_TIMEOUT`；先运行 `--structural` 确认基础链路 |
| 截图为空 | Chrome 未正确启动或被沙箱限制 | 检查 `SOULBROWSER_DISABLE_SANDBOX=1`（容器内）或 `SOULBROWSER_CHROME` 路径 |
| `perceiver` 无数据 | 尚未运行感知命令 | 先执行一次 `soulbrowser perceive --all`，再查看事件 |

## 与 RainbowBrowserAI 流程的映射

| RainbowBrowserAI 功能 | SoulBrowser 对应实现 |
|-----------------------|------------------------|
| Web UI 感知面板 (Lightning/Quick/Deep) | `soulbrowser perceive` CLI 模式（结构/视觉/全模态）与阶段 1 控制台原型 |
| Smart Element Search + Highlight | L8 视觉锚点与控制台高亮实验计划（脚本先行验证截图/洞察） |
| API `smart-element-search` | 现阶段通过 CLI；阶段 1 计划在 `docs/agent/` 定义 REST/SDK 接口 |
| 自动脚本 `test_perception_suite.sh` | `tests/l8_visual_suite.sh`（验证 JSON 字段、截图、洞察、事件摘要） |

## 后续扩展

- 随 Stage 1 落地视觉锚点后，补充对 `visual_anchor` 字段的校验与高亮确认脚本。
- 添加并发、错误回退等更多用例，确保 Agent 自愈策略的可视化反馈闭环。
- 将脚本集成进 CI，利用夜间真实浏览器环境跑感知回归测试。

---
在完成以上自动化与手动检查后，即可对 SoulBrowser 的感知/视觉链路建立信心，并为 L8 对话式 Agent 的视觉解释能力提供可靠的测试抓手。
