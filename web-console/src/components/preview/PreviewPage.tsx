import { useEffect, useRef } from 'react';
import { useParams } from 'react-router-dom';
import { Card, Timeline } from 'antd';
import { useScreenshotStore } from '@/stores/screenshotStore';
import { formatTime } from '@/utils/format';
import type { ElementOverlay } from '@/types';
import styles from './PreviewPage.module.css';

export default function PreviewPage() {
  const { taskId } = useParams<{ taskId: string }>();
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const { currentFrame, frames } = useScreenshotStore();

  const frame = taskId ? currentFrame.get(taskId) : undefined;
  const frameHistory = taskId ? frames.get(taskId) || [] : [];

  useEffect(() => {
    if (!frame || !canvasRef.current) return;

    const canvas = canvasRef.current;
    const ctx = canvas.getContext('2d');
    if (!ctx) return;

    const img = new Image();
    img.onload = () => {
      canvas.width = frame.viewport.width;
      canvas.height = frame.viewport.height;
      ctx.drawImage(img, 0, 0);

      // Draw overlays
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
      ctx.fillText(overlay.label, overlay.rect.x, overlay.rect.y - 5);
    }
  };

  return (
    <div className={styles.previewPage}>
      <div className={styles.preview}>
        <Card title="实时预览" className={styles.card}>
          <div className={styles.canvasContainer}>
            <canvas ref={canvasRef} className={styles.canvas} />
            {!frame && <div className={styles.noPreview}>暂无预览</div>}
          </div>
        </Card>
      </div>

      <div className={styles.sidebar}>
        <Card title="操作历史" className={styles.card}>
          <Timeline
            items={frameHistory.map((f, index) => ({
              children: (
                <div>
                  <div>{formatTime(f.timestamp)}</div>
                  <div className={styles.frameInfo}>
                    {f.overlays.length > 0 && `${f.overlays.length} 个元素高亮`}
                  </div>
                </div>
              ),
            }))}
          />
        </Card>
      </div>
    </div>
  );
}
