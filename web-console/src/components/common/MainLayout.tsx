import { useState } from 'react';
import { Outlet, useNavigate, useLocation } from 'react-router-dom';
import { Layout, Menu, Badge } from 'antd';
import {
  MessageOutlined,
  UnorderedListOutlined,
  DashboardOutlined,
  SettingOutlined,
} from '@ant-design/icons';
import { useTaskStore, selectRunningTasksCount } from '@/stores/taskStore';
import styles from './MainLayout.module.css';

const { Header, Sider, Content } = Layout;

export default function MainLayout() {
  const navigate = useNavigate();
  const location = useLocation();
  const [collapsed, setCollapsed] = useState(false);
  const runningTasksCount = useTaskStore(selectRunningTasksCount);

  const menuItems = [
    {
      key: '/chat',
      icon: <MessageOutlined />,
      label: '对话',
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
    {
      key: '/settings',
      icon: <SettingOutlined />,
      label: '设置',
    },
  ];

  return (
    <Layout className={styles.layout}>
      <Header className={styles.header}>
        <div className={styles.logo}>
          <span className={styles.logoIcon}>🤖</span>
          <span className={styles.logoText}>SoulBrowser</span>
        </div>
        <div className={styles.headerRight}>
          <span className={styles.version}>v1.0.0</span>
        </div>
      </Header>
      <Layout>
        <Sider
          collapsible
          collapsed={collapsed}
          onCollapse={setCollapsed}
          theme="dark"
          width={200}
          className={styles.sider}
        >
          <Menu
            mode="inline"
            selectedKeys={[location.pathname]}
            items={menuItems}
            onClick={({ key }) => navigate(key)}
            theme="dark"
          />
        </Sider>
        <Content className={styles.content}>
          <Outlet />
        </Content>
      </Layout>
    </Layout>
  );
}
