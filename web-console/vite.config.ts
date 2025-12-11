import { defineConfig, loadEnv } from 'vite';
import react from '@vitejs/plugin-react';
import path from 'path';

// https://vitejs.dev/config/
export default defineConfig(({ mode }) => {
  const env = loadEnv(mode, process.cwd(), '');

  const backendUrl = (env.VITE_BACKEND_URL || 'http://127.0.0.1:8804').replace(/\/$/, '');
  const backendWsUrl = env.VITE_BACKEND_WS_URL
    ? env.VITE_BACKEND_WS_URL.replace(/\/$/, '')
    : backendUrl.replace(/^http/i, backendUrl.startsWith('https') ? 'wss' : 'ws') + '/ws';
  const devPort = Number(env.VITE_DEV_PORT ?? 5173);

  return {
    plugins: [react()],
    resolve: {
      alias: {
        '@': path.resolve(__dirname, './src'),
      },
    },
    server: {
      port: devPort,
      proxy: {
        '/api': {
          target: backendUrl,
          changeOrigin: true,
        },
        '/ws': {
          target: backendWsUrl,
          ws: true,
          changeOrigin: true,
        },
      },
    },
    build: {
      outDir: 'dist',
      sourcemap: true,
      rollupOptions: {
        output: {
          manualChunks: {
            'react-vendor': ['react', 'react-dom', 'react-router-dom'],
            'antd-vendor': ['antd', '@ant-design/icons'],
            'chart-vendor': ['echarts', 'echarts-for-react'],
            'editor-vendor': ['monaco-editor', '@monaco-editor/react'],
          },
        },
      },
    },
  };
});
