import { useMemo, useState } from 'react';
import { Outlet, useNavigate, useLocation } from 'react-router-dom';
import { Layout, Menu, Badge, Space, Tag, Tooltip } from 'antd';
import type { MenuProps } from 'antd';
import {
  MessageOutlined,
  UnorderedListOutlined,
  DashboardOutlined,
  VideoCameraOutlined,
} from '@ant-design/icons';
import { useTaskStore, selectRunningTasksCount } from '@/stores/taskStore';
import styles from './MainLayout.module.css';

const { Header, Sider, Content } = Layout;

export default function MainLayout() {
  const navigate = useNavigate();
  const location = useLocation();
  const [collapsed, setCollapsed] = useState(false);
  const runningTasksCount = useTaskStore(selectRunningTasksCount);

  const menuItems: MenuProps['items'] = [
    {
      key: '/chat',
      icon: <MessageOutlined />,
      label: '对话',
    },
    {
      key: '/sessions',
      icon: <VideoCameraOutlined />,
      label: '会话',
    },
    {
      key: '/tasks',
      icon: (
        <Badge count={runningTasksCount} size="small" offset={[10, 0]}>
          <UnorderedListOutlined />
        </Badge>
      ),
      label: '任务',
    },
    {
      key: '/dashboard',
      icon: <DashboardOutlined />,
      label: '监控',
    },
  ];
  const selectedKey = ['/chat', '/sessions', '/tasks', '/dashboard'].find((key) =>
    location.pathname.startsWith(key)
  );

  const apiEndpoint = useMemo(() => {
    if (import.meta.env.VITE_API_URL) {
      return import.meta.env.VITE_API_URL;
    }
    if (typeof window !== 'undefined') {
      return window.location.origin;
    }
    return 'local';
  }, []);

  const plannerLabel = (import.meta.env.VITE_DEFAULT_PLANNER ?? 'llm').toUpperCase();

  return (
    <Layout className={styles.layout}>
      <Header className={styles.header}>
        <div className={styles.logo}>
          <span className={styles.logoIcon}>🤖</span>
          <span className={styles.logoText}>SoulBrowser</span>
        </div>
        <div className={styles.headerRight}>
          <Space size={8} className={styles.headerStatus}>
            <Tag color="cyan" className={styles.headerTag}>
              Planner&nbsp;{plannerLabel}
            </Tag>
            <Tooltip title={`API Endpoint: ${apiEndpoint}`} placement="bottom">
              <Tag color="geekblue" className={styles.headerTag}>
                API&nbsp;{apiEndpoint.replace(/^https?:\/\//, '')}
              </Tag>
            </Tooltip>
          </Space>
          <span className={styles.version}>Beta v1.0.0</span>
        </div>
      </Header>
      <Layout>
        <Sider
          collapsible
          collapsed={collapsed}
          onCollapse={setCollapsed}
          theme="light"
          width={220}
          className={styles.sider}
        >
          <Menu
            mode="inline"
            selectedKeys={[selectedKey ?? '/chat']}
            items={menuItems}
            onClick={({ key }) => navigate(key)}
            theme="light"
          />
        </Sider>
        <Content className={styles.content}>
          <div className={styles.contentInner}>
            <Outlet />
          </div>
        </Content>
      </Layout>
    </Layout>
  );
}
