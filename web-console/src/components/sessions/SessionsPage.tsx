import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { Button, Card, Empty, List, message, Space, Spin, Tag, Typography } from 'antd';
import {
  ReloadOutlined,
  ShareAltOutlined,
  VideoCameraOutlined,
  PlusOutlined,
} from '@ant-design/icons';
import dayjs from 'dayjs';
import relativeTime from 'dayjs/plugin/relativeTime';
import { soulbrowserAPI } from '@/api/soulbrowser';
import type {
  LiveFramePayload,
  LiveOverlayEntry,
  SessionLiveEvent,
  SessionRecord,
  SessionSnapshot,
} from '@/types';
import styles from './SessionsPage.module.css';

dayjs.extend(relativeTime);

const { Text } = Typography;

const STATUS_TEXT: Record<string, string> = {
  initializing: '初始化',
  active: '活跃',
  idle: '空闲',
  completed: '完成',
  failed: '异常',
};

const STATUS_COLOR: Record<string, string> = {
  initializing: 'default',
  active: 'success',
  idle: 'processing',
  completed: 'default',
  failed: 'error',
};

const OVERLAY_LIMIT = 12;

export default function SessionsPage() {
  const [sessions, setSessions] = useState<SessionRecord[]>([]);
  const [loading, setLoading] = useState(false);
  const [liveLoading, setLiveLoading] = useState(false);
  const [creating, setCreating] = useState(false);
  const [selectedSession, setSelectedSession] = useState<SessionRecord | null>(null);
  const [currentFrame, setCurrentFrame] = useState<LiveFramePayload | null>(null);
  const [overlayHistory, setOverlayHistory] = useState<LiveOverlayEntry[]>([]);
  const eventSourceRef = useRef<EventSource | null>(null);
  const selectedSessionIdRef = useRef<string | null>(null);

  const applySnapshot = useCallback((snapshot: SessionSnapshot) => {
    setCurrentFrame(snapshot.last_frame ?? null);
    setOverlayHistory(snapshot.overlays ?? []);
    setSessions((prev) => {
      let replaced = false;
      const next = prev.map((session) => {
        if (session.id === snapshot.session.id) {
          replaced = true;
          return snapshot.session;
        }
        return session;
      });
      if (!replaced) {
        return [snapshot.session, ...next];
      }
      return next;
    });
  }, []);

  const attachStream = useCallback(
    (sessionId: string, shareToken?: string) => {
      eventSourceRef.current?.close();
      const source = soulbrowserAPI.openSessionStream(sessionId, shareToken);
      const handler = (event: MessageEvent) => {
        try {
          const payload = JSON.parse(event.data) as SessionLiveEvent;
          switch (payload.type) {
            case 'snapshot':
              applySnapshot(payload.snapshot);
              break;
            case 'frame':
              setCurrentFrame(payload.frame);
              break;
            case 'overlay':
              setOverlayHistory((prev) => {
                const next = [...prev, payload.overlay];
                return next.slice(-OVERLAY_LIMIT);
              });
              break;
            case 'status':
              setSessions((prev) =>
                prev.map((item) =>
                  item.id === payload.session_id
                    ? { ...item, status: payload.status, updated_at: new Date().toISOString() }
                    : item
                )
              );
              break;
            default:
              break;
          }
        } catch (err) {
          console.warn('invalid live event', err);
        }
      };
      source.addEventListener('snapshot', handler as EventListener);
      source.addEventListener('frame', handler as EventListener);
      source.addEventListener('overlay', handler as EventListener);
      source.addEventListener('status', handler as EventListener);
      source.onerror = () => {
        message.warning('实时流已断开');
      };
      eventSourceRef.current = source;
    },
    [applySnapshot]
  );

  const selectSession = useCallback(
    async (session: SessionRecord) => {
      setSelectedSession(session);
      setLiveLoading(true);
      try {
        const snapshot = await soulbrowserAPI.getSessionSnapshot(session.id);
        applySnapshot(snapshot);
        attachStream(session.id, session.share_token);
      } catch (err) {
        console.error(err);
        message.error('加载会话详情失败');
      } finally {
        setLiveLoading(false);
      }
    },
    [attachStream, applySnapshot]
  );

  const refreshSessions = useCallback(async () => {
    setLoading(true);
    try {
      const list = await soulbrowserAPI.listSessions();
      setSessions(list);
      const selectedId = selectedSessionIdRef.current;
      if (!selectedId && list.length > 0) {
        await selectSession(list[0]);
      } else if (selectedId) {
        const updated = list.find((item) => item.id === selectedId);
        if (updated) {
          setSelectedSession(updated);
        }
      }
    } catch (err) {
      console.error(err);
      message.error('加载会话失败');
    } finally {
      setLoading(false);
    }
  }, [selectSession]);

  useEffect(() => {
    selectedSessionIdRef.current = selectedSession?.id ?? null;
  }, [selectedSession]);

  useEffect(() => {
    void refreshSessions();
    return () => {
      eventSourceRef.current?.close();
      eventSourceRef.current = null;
    };
  }, [refreshSessions]);

  const handleCreateSession = async () => {
    setCreating(true);
    try {
      const record = await soulbrowserAPI.createSession();
      message.success('已创建新会话');
      await selectSession(record);
      await refreshSessions();
    } catch (err) {
      console.error(err);
      message.error('创建会话失败');
    } finally {
      setCreating(false);
    }
  };

  const handleShareToggle = async () => {
    if (!selectedSession) return;
    try {
      if (selectedSession.share_token) {
        await soulbrowserAPI.revokeSessionShare(selectedSession.id);
        message.success('已关闭分享链接');
      } else {
        await soulbrowserAPI.issueSessionShare(selectedSession.id);
        message.success('已生成临时分享链接');
      }
      await refreshSessions();
    } catch (err) {
      console.error(err);
      message.error('更新分享状态失败');
    }
  };

  const statusTag = (status: string) => (
    <Tag color={STATUS_COLOR[status] || 'default'}>{STATUS_TEXT[status] ?? status}</Tag>
  );

  const shareUrl = useMemo(() => {
    if (!selectedSession) return null;
    if (selectedSession.share_url) return selectedSession.share_url;
    if (selectedSession.share_path) {
      return `${window.location.origin}${selectedSession.share_path}`;
    }
    return null;
  }, [selectedSession]);

  return (
    <div className={styles.sessionsPage}>
      <div className={styles.listPanel}>
        <Card
          title={
            <Space>
              <VideoCameraOutlined />
              <span>持久会话</span>
            </Space>
          }
          extra={
            <Space>
              <Button
                size="small"
                icon={<ReloadOutlined />}
                onClick={refreshSessions}
                loading={loading}
              >
                刷新
              </Button>
            </Space>
          }
          bodyStyle={{ padding: 0 }}
        >
          <List
            loading={loading}
            dataSource={sessions}
            rowKey={(session) => session.id}
            locale={{ emptyText: <Empty description="暂无会话" /> }}
            renderItem={(session) => (
              <List.Item
                onClick={() => selectSession(session)}
                style={{
                  cursor: 'pointer',
                  background: selectedSession?.id === session.id ? 'rgba(111, 77, 255, 0.08)' : 'transparent',
                  padding: '12px 16px',
                }}
              >
                <div className={styles.sessionItem}>
                  <Space>
                    <Text strong>{session.profile_label || session.id.slice(0, 8)}</Text>
                    {statusTag(session.status)}
                  </Space>
                  <div className={styles.sessionMeta}>
                    最近活跃：{session.updated_at ? dayjs(session.updated_at).fromNow() : '未知'}
                  </div>
                </div>
              </List.Item>
            )}
          />
        </Card>
        <Button
          type="primary"
          icon={<PlusOutlined />}
          onClick={handleCreateSession}
          loading={creating}
        >
          创建持久会话
        </Button>
      </div>
      <div className={styles.viewerPanel}>
        <Card
          title={selectedSession ? `实时画面 · ${selectedSession.id.slice(0, 8)}` : '实时画面'}
          extra={
            selectedSession && (
              <Space className={styles.sessionActions}>
                <Button size="small" icon={<ShareAltOutlined />} onClick={handleShareToggle}>
                  {selectedSession.share_token ? '收回分享' : '生成分享'}
                </Button>
              </Space>
            )
          }
        >
          <div className={styles.viewerCanvas}>
            {currentFrame ? (
              <img
                src={`data:image/png;base64,${currentFrame.screenshot_base64}`}
                alt="live screenshot"
              />
            ) : (
              <div className={styles.emptyState}>
                {liveLoading ? <Spin tip="等待最新画面" /> : '选择会话以查看实时画面'}
              </div>
            )}
          </div>
          {shareUrl && (
            <div style={{ marginTop: 12 }}>
              <Text type="secondary">分享链接：</Text>
              <Text copyable>{shareUrl}</Text>
            </div>
          )}
        </Card>
        <Card title="最新事件">
          <List
            className={styles.overlayList}
            dataSource={[...overlayHistory].reverse()}
            rowKey={(entry) =>
              `${entry.session_id}-${entry.recorded_at}-${entry.data?.action_id ?? entry.task_id ?? 'overlay'}`
            }
            locale={{ emptyText: <Empty description="暂无事件" /> }}
            renderItem={(entry) => (
              <List.Item>
                <div className={styles.overlayItem}>
                  <Text strong>{entry.data?.label || entry.source}</Text>
                  <div className={styles.overlayMeta}>
                    {dayjs(entry.recorded_at).format('HH:mm:ss')} · 工具{' '}
                    {entry.data?.dispatch_label || entry.task_id || '未知'}
                  </div>
                </div>
              </List.Item>
            )}
          />
        </Card>
      </div>
    </div>
  );
}
