import { useState, useEffect } from 'react';
import { Space, Tag, Typography, Button, Tooltip, Modal, Input, message } from 'antd';
import { ReloadOutlined, LinkOutlined } from '@ant-design/icons';
import { useBackendConfigStore, type BackendStatus } from '@/stores/backendConfigStore';
import styles from './BackendStatusBar.module.css';

interface Props {
  className?: string;
}

const STATUS_MAP: Record<BackendStatus, { color: string; label: string }> = {
  online: { color: 'green', label: '后端在线' },
  offline: { color: 'red', label: '无法连接' },
  checking: { color: 'blue', label: '检测中' },
  unknown: { color: 'default', label: '未知状态' },
};

export default function BackendStatusBar({ className }: Props) {
  const { baseUrl, status, lastChecked, setBaseUrl, checkBackend } = useBackendConfigStore();
  const [modalOpen, setModalOpen] = useState(false);
  const [value, setValue] = useState(baseUrl);

  useEffect(() => {
    setValue(baseUrl);
  }, [baseUrl]);

  useEffect(() => {
    if (status === 'unknown') {
      void checkBackend();
    }
  }, [status, checkBackend]);

  const handleChange = () => {
    const next = value.trim();
    if (!next) {
      message.warning('后端地址不能为空');
      return;
    }
    setBaseUrl(next);
    setModalOpen(false);
    message.success('已更新后端地址');
  };

  const meta = STATUS_MAP[status] ?? STATUS_MAP.unknown;

  return (
    <div className={[styles.wrapper, className].filter(Boolean).join(' ')}>
      <div className={styles.statusRow}>
        <Space size="small">
          <Tag color={meta.color}>{meta.label}</Tag>
          <Typography.Text className={styles.urlText}>{baseUrl}</Typography.Text>
        </Space>
        <Space>
          <Tooltip title="重新检测">
            <Button
              icon={<ReloadOutlined />}
              size="small"
              loading={status === 'checking'}
              onClick={() => checkBackend()}
            />
          </Tooltip>
          <Button
            icon={<LinkOutlined />}
            size="small"
            onClick={() => setModalOpen(true)}
          >
            切换后端
          </Button>
        </Space>
      </div>
      {lastChecked && (
        <Typography.Text type="secondary" className={styles.timestamp}>
          上次检测：{new Date(lastChecked).toLocaleString()}
        </Typography.Text>
      )}

      <Modal
        title="切换后端地址"
        open={modalOpen}
        onOk={handleChange}
        onCancel={() => setModalOpen(false)}
        okText="保存"
        cancelText="取消"
      >
        <Input
          value={value}
          onChange={(e) => setValue(e.target.value)}
          placeholder="http://127.0.0.1:8804"
          autoFocus
        />
      </Modal>
    </div>
  );
}
