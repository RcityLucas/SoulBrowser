# SoulBrowser 前端开发环境搭建指南

本指南将帮助你快速搭建和运行 SoulBrowser Web Console 前端项目。

## 前置要求

- Node.js >= 18.0.0
- npm >= 9.0.0 或 pnpm >= 8.0.0
- 现代浏览器（Chrome、Edge、Firefox 等）

## 安装步骤

### 1. 进入项目目录

```bash
cd /mnt/d/github/SoulBrowserClaude/SoulBrowser/web-console
```

### 2. 安装依赖

使用 npm:
```bash
npm install
```

或使用 pnpm（推荐，更快）:
```bash
pnpm install
```

### 3. 启动开发服务器

```bash
npm run dev
```

服务器将在 http://localhost:5173 启动。

## 开发模式

### 热更新

Vite 提供了快速的 HMR（热模块替换），修改代码后浏览器会自动更新，无需手动刷新。

### 类型检查

运行 TypeScript 类型检查：

```bash
npm run type-check
```

### 代码检查

运行 ESLint 检查：

```bash
npm run lint
```

## 与后端联调

### 1. 启动后端服务

在另一个终端窗口中，启动 Rust 后端：

```bash
cd /mnt/d/github/SoulBrowserClaude/SoulBrowser
cargo run --bin soulbrowser -- serve --port 8080
```

### 2. 代理配置

Vite 已配置了开发代理，将自动转发 API 和 WebSocket 请求到后端：

- HTTP API: `/api/*` → `http://localhost:8080`
- WebSocket: `/ws` → `ws://localhost:8080/ws`

### 3. 测试连接

打开浏览器开发者工具，查看 Network 和 Console 标签，确认：
- HTTP 请求正常返回
- WebSocket 连接成功建立

## 生产构建

### 构建项目

```bash
npm run build
```

构建产物位于 `dist/` 目录。

### 预览构建结果

```bash
npm run preview
```

## 常见问题

### 1. 依赖安装失败

**问题**: `npm install` 报错

**解决方案**:
```bash
# 清理缓存
npm cache clean --force

# 删除 node_modules 和 package-lock.json
rm -rf node_modules package-lock.json

# 重新安装
npm install
```

### 2. 端口被占用

**问题**: 5173 端口已被使用

**解决方案**:
修改 `vite.config.ts`：
```typescript
export default defineConfig({
  server: {
    port: 3000, // 改为其他端口
  },
});
```

### 3. WebSocket 连接失败

**问题**: 控制台显示 WebSocket 连接错误

**解决方案**:
1. 确认后端服务已启动
2. 检查后端端口是否为 8080
3. 查看 `vite.config.ts` 中的代理配置是否正确

### 4. TypeScript 错误

**问题**: 编辑器显示大量 TypeScript 错误

**解决方案**:
```bash
# 重新安装类型定义
npm install --save-dev @types/react @types/react-dom

# 重启编辑器的 TypeScript 服务
# VSCode: Ctrl+Shift+P -> "TypeScript: Restart TS Server"
```

### 5. 样式不生效

**问题**: CSS 模块样式没有应用

**解决方案**:
1. 确保 CSS 文件以 `.module.css` 结尾
2. 正确导入: `import styles from './Component.module.css'`
3. 使用: `className={styles.className}`

## 开发技巧

### 1. 快速导航

使用 VSCode 的快捷键：
- `Ctrl+P`: 快速打开文件
- `Ctrl+Shift+F`: 全局搜索
- `F12`: 跳转到定义
- `Alt+Left/Right`: 前进/后退

### 2. 调试技巧

在浏览器开发者工具中：
- **Elements**: 检查 DOM 结构和样式
- **Console**: 查看日志和错误
- **Network**: 监控 HTTP 和 WebSocket 请求
- **Sources**: 设置断点调试

### 3. 性能优化

- 使用 React DevTools 分析组件渲染
- 使用 Chrome Lighthouse 检查性能
- 使用 `React.memo()` 避免不必要的重渲染
- 使用 `useMemo()` 和 `useCallback()` 优化计算和回调

### 4. 代码格式化

安装 Prettier（可选）:
```bash
npm install --save-dev prettier
```

创建 `.prettierrc`:
```json
{
  "semi": true,
  "singleQuote": true,
  "tabWidth": 2,
  "printWidth": 100
}
```

## 下一步

- 阅读 [UI_DEVELOPMENT_PLAN.md](./UI_DEVELOPMENT_PLAN.md) 了解详细的开发计划
- 查看 [web-console/README.md](../web-console/README.md) 了解项目结构
- 开始开发新功能或修复 bug

## 需要帮助？

- 查看 TypeScript 文档: https://www.typescriptlang.org/docs/
- 查看 React 文档: https://react.dev/
- 查看 Ant Design 文档: https://ant.design/
- 查看 Vite 文档: https://vitejs.dev/
