import { BrowserRouter, Routes, Route, Navigate } from 'react-router-dom';
import { ConfigProvider, theme } from 'antd';
import zhCN from 'antd/locale/zh_CN';
import MainLayout from './components/common/MainLayout';
import ChatPage from './components/chat/ChatPage';
import TasksPage from './components/tasks/TasksPage';
import PreviewPage from './components/preview/PreviewPage';
import DashboardPage from './components/dashboard/DashboardPage';
import SessionsPage from './components/sessions/SessionsPage';

function App() {
  return (
    <BrowserRouter>
      <ConfigProvider
        locale={zhCN}
        theme={{
          algorithm: theme.defaultAlgorithm,
          token: {
            colorPrimary: '#6f4dff',
            colorInfo: '#6f4dff',
            colorSuccess: '#2db984',
            colorWarning: '#ffb347',
            colorBgLayout: 'transparent',
            colorBgContainer: 'rgba(255, 255, 255, 0.85)',
            colorBorder: '#dfe6ff',
            colorTextBase: '#1b1e2b',
            borderRadius: 14,
            controlHeight: 42,
            fontFamily: 'Inter, Segoe UI, sans-serif',
          },
        }}
      >
        <Routes>
            <Route path="/" element={<MainLayout />}>
              <Route index element={<Navigate to="/chat" replace />} />
              <Route path="chat" element={<ChatPage />} />
              <Route path="sessions" element={<SessionsPage />} />
              <Route path="tasks" element={<TasksPage />} />
            <Route path="tasks/:taskId" element={<PreviewPage />} />
            <Route path="dashboard" element={<DashboardPage />} />
          </Route>
        </Routes>
      </ConfigProvider>
    </BrowserRouter>
  );
}

export default App;
