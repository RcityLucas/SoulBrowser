import { useEffect, useMemo, useRef, useState } from 'react';
import { Card, Empty, List, Space, Spin, Tag, Typography, message } from 'antd';
import type {
  LiveFramePayload,
  LiveOverlayEntry,
  SessionLiveEvent,
  SessionRecord,
} from '@/types';
import { soulbrowserAPI } from '@/api/soulbrowser';
import { formatTime } from '@/utils/format';
import styles from './LiveSessionPreview.module.css';

const { Text } = Typography;
const MAX_OVERLAY_HISTORY = 10;

interface LiveSessionPreviewProps {
  sessionId: string | null;
  session?: SessionRecord;
  onSessionSnapshot?: (sessionId: string, hasFrame: boolean) => void;
}

export default function LiveSessionPreview({
  sessionId,
  session,
  onSessionSnapshot,
}: LiveSessionPreviewProps) {
  const [frame, setFrame] = useState<LiveFramePayload | null>(null);
  const [overlays, setOverlays] = useState<LiveOverlayEntry[]>([]);
  const [connecting, setConnecting] = useState(false);
  const streamRef = useRef<EventSource | null>(null);

  const statusMeta = useMemo(() => {
    if (!session) {
      return null;
    }
    switch (session.status) {
      case 'active':
        return { color: 'success', label: '活跃' };
      case 'idle':
        return { color: 'processing', label: '空闲' };
      case 'completed':
        return { color: 'default', label: '完成' };
      case 'failed':
        return { color: 'error', label: '异常' };
      case 'initializing':
      default:
        return { color: 'default', label: '初始化' };
    }
  }, [session]);

  useEffect(() => {
    streamRef.current?.close();
    streamRef.current = null;
    setFrame(null);
    setOverlays([]);
    if (!sessionId) {
      return;
    }

    let cancelled = false;

    const notifySnapshot = (hasFrame: boolean) => {
      if (sessionId && onSessionSnapshot) {
        onSessionSnapshot(sessionId, hasFrame);
      }
    };

    const bootstrap = async () => {
      setConnecting(true);
      try {
        const snapshot = await soulbrowserAPI.getSessionSnapshot(sessionId);
        if (cancelled) {
          return;
        }
        setFrame(snapshot.last_frame ?? null);
        setOverlays(snapshot.overlays ?? []);
        notifySnapshot(Boolean(snapshot.last_frame));
      } catch (err) {
        if (!cancelled) {
          console.error(err);
          message.error('加载实时画面失败');
        }
      } finally {
        if (!cancelled) {
          setConnecting(false);
        }
      }
    };

    void bootstrap();

    const source = soulbrowserAPI.openSessionStream(sessionId, session?.share_token);
    const listener = (event: MessageEvent) => {
      try {
        const payload = JSON.parse(event.data) as SessionLiveEvent;
        switch (payload.type) {
          case 'snapshot':
            setFrame(payload.snapshot.last_frame ?? null);
            setOverlays(payload.snapshot.overlays ?? []);
            notifySnapshot(Boolean(payload.snapshot.last_frame));
            break;
          case 'frame':
            setFrame(payload.frame);
            if (payload.frame.overlays?.length) {
              setOverlays((prev) =>
                [...prev, ...payload.frame.overlays!].slice(-MAX_OVERLAY_HISTORY)
              );
            }
            notifySnapshot(true);
            break;
          case 'overlay':
            setOverlays((prev) => [...prev, payload.overlay].slice(-MAX_OVERLAY_HISTORY));
            break;
          case 'status':
          default:
            break;
        }
      } catch (err) {
        console.warn('invalid live session event', err);
      }
    };

    source.addEventListener('snapshot', listener as EventListener);
    source.addEventListener('frame', listener as EventListener);
    source.addEventListener('overlay', listener as EventListener);
    source.onerror = () => {
      message.warning('实时画面连接已断开');
    };

    streamRef.current = source;

    return () => {
      cancelled = true;
      source.removeEventListener('snapshot', listener as EventListener);
      source.removeEventListener('frame', listener as EventListener);
      source.removeEventListener('overlay', listener as EventListener);
      source.close();
    };
  }, [sessionId, session?.share_token, onSessionSnapshot]);

  const overlayItems = useMemo(() => [...overlays].reverse(), [overlays]);

  return (
    <div className={styles.previewPanel}>
      <Card
        size="small"
        title="实时画面"
        className={styles.previewCard}
        extra={
          session && (
            <Space size={8} align="center">
              {statusMeta && <Tag color={statusMeta.color}>{statusMeta.label}</Tag>}
              <Text type="secondary" className={styles.sessionId} copyable>
                {session.id.slice(0, 12)}
              </Text>
            </Space>
          )
        }
      >
        {!sessionId ? (
          <div className={styles.placeholder}>选择或创建会话即可看到执行画面</div>
        ) : (
          <div className={styles.canvas}>
            {frame ? (
              <img
                src={`data:image/png;base64,${frame.screenshot_base64}`}
                alt="session preview"
              />
            ) : connecting ? (
              <Spin tip="等待最新画面" />
            ) : (
              <Empty description="暂无画面" image={Empty.PRESENTED_IMAGE_SIMPLE} />
            )}
          </div>
        )}
        {frame?.route?.page && (
          <div className={styles.routeMeta}>
            当前页面：
            <Text copyable className={styles.routeText}>
              {frame.route.page}
            </Text>
          </div>
        )}
      </Card>

      <Card size="small" title="最新事件" className={styles.previewCard}>
        {sessionId ? (
          <List
            size="small"
            className={styles.overlayList}
            locale={{ emptyText: '暂无事件' }}
            dataSource={overlayItems}
            renderItem={(entry) => (
              <List.Item>
                <div className={styles.overlayItem}>
                  <Space direction="vertical" size={2}>
                    <Space size={6} align="center">
                      <Tag color={entry.source === 'execution' ? 'volcano' : 'geekblue'}>
                        {entry.source === 'execution' ? '执行' : '计划'}
                      </Tag>
                      <Text>{entry.data?.label || entry.data?.dispatch_label || '事件'}</Text>
                    </Space>
                    <Text type="secondary" className={styles.overlayMeta}>
                      {formatTime(entry.recorded_at)} · {entry.data?.dispatch_label || entry.task_id}
                    </Text>
                    {entry.data?.detail && (
                      <Text type="secondary" className={styles.overlayMeta}>
                        {entry.data.detail}
                      </Text>
                    )}
                  </Space>
                </div>
              </List.Item>
            )}
          />
        ) : (
          <div className={styles.placeholder}>会话创建后会显示操作事件</div>
        )}
      </Card>
    </div>
  );
}
