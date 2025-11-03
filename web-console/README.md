# SoulBrowser Web Console

SoulBrowser 的 Web 可视化控制台，提供直观的任务管理和监控界面。

## 功能特性

### ✨ 核心功能

- **对话式交互** - 通过自然语言创建和管理自动化任务
- **任务管理** - 实时监控任务执行状态和进度
- **实时预览** - 查看浏览器截图和操作轨迹
- **监控仪表盘** - 任务统计和性能指标可视化

### 🎨 界面特点

- 现代化暗色主题
- 响应式布局
- 实时 WebSocket 通信
- 流畅的动画效果

## 技术栈

- **前端框架**: React 18 + TypeScript
- **UI 组件**: Ant Design 5
- **状态管理**: Zustand
- **图表库**: ECharts
- **构建工具**: Vite
- **实时通信**: WebSocket

## 快速开始

### 安装依赖

```bash
cd web-console
npm install
```

### 开发模式

```bash
npm run dev
```

访问 http://localhost:5173

### 生产构建

```bash
npm run build
```

构建产物位于 `dist/` 目录。

## 项目结构

```
web-console/
├── src/
│   ├── api/              # API 客户端
│   │   ├── client.ts     # HTTP 客户端
│   │   └── websocket.ts  # WebSocket 客户端
│   ├── components/       # React 组件
│   │   ├── common/       # 通用组件
│   │   ├── chat/         # 对话界面
│   │   ├── tasks/        # 任务管理
│   │   ├── preview/      # 实时预览
│   │   └── dashboard/    # 监控仪表盘
│   ├── stores/           # Zustand 状态管理
│   ├── hooks/            # 自定义 Hooks
│   ├── types/            # TypeScript 类型定义
│   ├── utils/            # 工具函数
│   ├── App.tsx           # 主应用组件
│   └── main.tsx          # 入口文件
├── package.json
├── tsconfig.json
├── vite.config.ts
└── index.html
```

## 开发指南

### 添加新页面

1. 在 `src/components/` 下创建页面组件
2. 在 `App.tsx` 中添加路由
3. 在 `MainLayout.tsx` 中添加菜单项

### 状态管理

使用 Zustand 管理全局状态：

```typescript
// 创建 store
export const useMyStore = create<MyState>()((set) => ({
  data: [],
  setData: (data) => set({ data }),
}));

// 使用 store
function MyComponent() {
  const { data, setData } = useMyStore();
  // ...
}
```

### WebSocket 通信

```typescript
import { useWebSocket } from '@/hooks/useWebSocket';

function MyComponent() {
  const { send, on } = useWebSocket();

  useEffect(() => {
    // 订阅事件
    const unsubscribe = on('my_event', (data) => {
      console.log('Received:', data);
    });

    return unsubscribe;
  }, [on]);

  // 发送消息
  const handleSend = () => {
    send({ type: 'my_message', payload: {} });
  };
}
```

## 配置说明

### 环境变量

创建 `.env` 文件：

```env
# API 地址
VITE_API_URL=http://localhost:8080

# WebSocket 地址
VITE_WS_URL=ws://localhost:8080/ws
```

### Vite 代理配置

在 `vite.config.ts` 中配置开发服务器代理：

```typescript
export default defineConfig({
  server: {
    proxy: {
      '/api': 'http://localhost:8080',
      '/ws': {
        target: 'ws://localhost:8080',
        ws: true,
      },
    },
  },
});
```

## 部署

### Docker 部署

```dockerfile
FROM node:18-alpine AS builder
WORKDIR /app
COPY package*.json ./
RUN npm ci
COPY . .
RUN npm run build

FROM nginx:alpine
COPY --from=builder /app/dist /usr/share/nginx/html
COPY nginx.conf /etc/nginx/conf.d/default.conf
EXPOSE 80
CMD ["nginx", "-g", "daemon off;"]
```

### Nginx 配置

```nginx
server {
  listen 80;
  server_name _;

  root /usr/share/nginx/html;
  index index.html;

  location / {
    try_files $uri $uri/ /index.html;
  }

  location /api/ {
    proxy_pass http://backend:8080;
    proxy_set_header Host $host;
  }

  location /ws {
    proxy_pass http://backend:8080;
    proxy_http_version 1.1;
    proxy_set_header Upgrade $http_upgrade;
    proxy_set_header Connection "upgrade";
  }
}
```

## 常见问题

### WebSocket 连接失败

确保后端服务已启动并监听在正确的端口。检查 Vite 代理配置是否正确。

### 样式不生效

确保正确导入了 CSS 模块文件，并使用 `styles.className` 的方式引用样式。

### TypeScript 类型错误

运行 `npm run type-check` 检查类型错误，确保所有类型定义正确。

## License

MIT
