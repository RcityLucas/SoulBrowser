import { BrowserRouter, Routes, Route, Navigate } from 'react-router-dom';
import { ConfigProvider, theme } from 'antd';
import zhCN from 'antd/locale/zh_CN';
import MainLayout from './components/common/MainLayout';
import ChatPage from './components/chat/ChatPage';
import TasksPage from './components/tasks/TasksPage';
import PreviewPage from './components/preview/PreviewPage';
import DashboardPage from './components/dashboard/DashboardPage';

function App() {
  return (
    <BrowserRouter>
      <ConfigProvider
        locale={zhCN}
        theme={{
          algorithm: theme.darkAlgorithm,
          token: {
            colorPrimary: '#1890ff',
            borderRadius: 6,
          },
        }}
      >
        <Routes>
          <Route path="/" element={<MainLayout />}>
            <Route index element={<Navigate to="/chat" replace />} />
            <Route path="chat" element={<ChatPage />} />
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
