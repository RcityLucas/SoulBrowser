import { useEffect, useMemo, useRef, useState } from 'react';
import { useParams } from 'react-router-dom';
import { Alert, Card, Spin, Timeline, Typography } from 'antd';
import type { LiveFramePayload, LiveOverlayEntry, SessionLiveEvent } from '@/types';
import { useScreenshotStore } from '@/stores/screenshotStore';
import { soulbrowserAPI } from '@/api/soulbrowser';
import { formatTime } from '@/utils/format';
import type { ElementOverlay, ScreenshotFrame } from '@/types';
import styles from './PreviewPage.module.css';

const DEFAULT_VIEWPORT = { width: 1280, height: 720, deviceScaleFactor: 1 };

export default function PreviewPage() {
  const { taskId } = useParams<{ taskId: string }>();
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const eventSourceRef = useRef<EventSource | null>(null);
  const { currentFrame, frames, addFrame, clearFrames } = useScreenshotStore();
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [sessionInfo, setSessionInfo] = useState<{ id: string; status?: string } | null>(null);

  const frame = taskId ? currentFrame.get(taskId) : undefined;
  const frameHistory = useMemo(() => (taskId ? frames.get(taskId) || [] : []), [frames, taskId]);

  useEffect(() => {
    if (!taskId) {
      return undefined;
    }

    let cancelled = false;

    const attachStream = async () => {
      setError(null);
      setLoading(true);
      clearFrames(taskId);
      eventSourceRef.current?.close();

      try {
        const detail = await soulbrowserAPI.getTask(taskId);
        const sessionId: string | undefined =
          detail.task?.session_id ?? detail.task?.plan?.session_id ?? detail.task?.flow?.session_id;

        if (!sessionId) {
          throw new Error('任务没有关联的实时浏览器会话');
        }

        const snapshot = await soulbrowserAPI.getSessionSnapshot(sessionId);
        if (cancelled) return;

        setSessionInfo({ id: sessionId, status: snapshot.session.status });

        if (snapshot.last_frame) {
          addFrame(taskId, mapFramePayload(taskId, snapshot.last_frame));
        }

        const source = soulbrowserAPI.openSessionStream(sessionId, snapshot.session.share_token);

        const handleEvent = (event: MessageEvent) => {
          try {
            const payload = JSON.parse(event.data) as SessionLiveEvent;
            if (payload.type === 'frame') {
              addFrame(taskId, mapFramePayload(taskId, payload.frame));
            } else if (payload.type === 'snapshot') {
              setSessionInfo({ id: payload.snapshot.session.id, status: payload.snapshot.session.status });
              if (payload.snapshot.last_frame) {
                addFrame(taskId, mapFramePayload(taskId, payload.snapshot.last_frame));
              }
            } else if (payload.type === 'status') {
              setSessionInfo((prev) =>
                prev && prev.id === payload.session_id ? { ...prev, status: payload.status } : prev
              );
            }
          } catch (err) {
            console.warn('invalid live preview event', err);
          }
        };

        source.addEventListener('frame', handleEvent as EventListener);
        source.addEventListener('snapshot', handleEvent as EventListener);
        source.addEventListener('status', handleEvent as EventListener);
        source.onerror = () => {
          setError('实时流已断开联系');
        };

        eventSourceRef.current = source;
      } catch (err) {
        const message = err instanceof Error ? err.message : '加载实时预览失败';
        setError(message);
      } finally {
        if (!cancelled) {
          setLoading(false);
        }
      }
    };

    attachStream();

    return () => {
      cancelled = true;
      eventSourceRef.current?.close();
      eventSourceRef.current = null;
      clearFrames(taskId);
    };
  }, [addFrame, clearFrames, taskId]);

  useEffect(() => {
    if (!frame || !canvasRef.current) return;

    const canvas = canvasRef.current;
    const ctx = canvas.getContext('2d');
    if (!ctx) return;

    const img = new Image();
    img.onload = () => {
      const width = img.naturalWidth || frame.viewport.width || DEFAULT_VIEWPORT.width;
      const height = img.naturalHeight || frame.viewport.height || DEFAULT_VIEWPORT.height;
      canvas.width = width;
      canvas.height = height;
      ctx.clearRect(0, 0, width, height);
      ctx.drawImage(img, 0, 0, width, height);

      frame.overlays.forEach((overlay) => {
        drawOverlay(ctx, overlay);
      });
    };
    img.src = `data:image/png;base64,${frame.data}`;
  }, [frame]);

  const drawOverlay = (ctx: CanvasRenderingContext2D, overlay: ElementOverlay) => {
    ctx.strokeStyle = overlay.color || '#00ff00';
    ctx.lineWidth = 2;
    ctx.strokeRect(overlay.rect.x, overlay.rect.y, overlay.rect.width, overlay.rect.height);

    if (overlay.label) {
      ctx.fillStyle = overlay.color || '#00ff00';
      ctx.font = '14px Arial';
      ctx.fillText(overlay.label, overlay.rect.x, Math.max(0, overlay.rect.y - 5));
    }
  };

  return (
    <div className={styles.previewPage}>
      <div className={styles.preview}>
        <Card
          title={
            <div className={styles.previewHeader}>
              <span>实时预览</span>
              {sessionInfo && (
                <Typography.Text type="secondary">
                  会话 {sessionInfo.id} · 状态 {sessionInfo.status ?? '未知'}
                </Typography.Text>
              )}
            </div>
          }
          className={styles.card}
        >
          <div className={styles.canvasContainer}>
            <canvas ref={canvasRef} className={styles.canvas} />
            {loading && (
              <div className={styles.overlayMessage}>
                <Spin />
              </div>
            )}
            {!loading && !frame && !error && <div className={styles.noPreview}>暂无实时画面</div>}
            {error && (
              <div className={styles.overlayMessage}>
                <Alert type="error" message={error} showIcon />
              </div>
            )}
          </div>
        </Card>
      </div>

      <div className={styles.sidebar}>
        <Card title="操作历史" className={styles.card}>
          {frameHistory.length ? (
            <Timeline
              items={frameHistory.map((f) => ({
                children: (
                  <div>
                    <div>{formatTime(f.timestamp)}</div>
                    <div className={styles.frameInfo}>
                      {f.overlays.length > 0 ? `${f.overlays.length} 个元素高亮` : '截图捕获'}
                    </div>
                  </div>
                ),
              }))}
            />
          ) : (
            <div className={styles.emptyTimeline}>暂无历史</div>
          )}
        </Card>
      </div>
    </div>
  );
}

function mapFramePayload(taskId: string, payload: LiveFramePayload): ScreenshotFrame {
  return {
    taskId,
    timestamp: new Date(payload.recorded_at),
    data: payload.screenshot_base64,
    overlays: mapOverlays(payload.overlays),
    viewport: DEFAULT_VIEWPORT,
  };
}

function mapOverlays(entries?: LiveOverlayEntry[]): ElementOverlay[] {
  if (!entries || entries.length === 0) {
    return [];
  }

  return entries
    .map((entry, index) => {
      const bbox = (entry.data as Record<string, any>)?.bbox as Record<string, number> | undefined;
      if (!bbox) {
        return null;
      }

      const rect = {
        x: bbox.x ?? bbox.left ?? 0,
        y: bbox.y ?? bbox.top ?? 0,
        width: bbox.width ?? (bbox.right ?? 0) - (bbox.left ?? 0),
        height: bbox.height ?? (bbox.bottom ?? 0) - (bbox.top ?? 0),
      };

      if (rect.width <= 0 || rect.height <= 0) {
        return null;
      }

      return {
        id: `${entry.session_id}-${index}`,
        type: 'highlight' as const,
        rect,
        label:
          (entry.data && (entry.data.label ?? entry.data.detail ?? entry.data.step_id)) ||
          undefined,
        color: '#52c41a',
      } satisfies ElementOverlay;
    })
    .filter((overlay): overlay is ElementOverlay => Boolean(overlay));
}
