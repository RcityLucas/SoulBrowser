import { Empty, List, Space, Tag, Typography } from 'antd';
import dayjs from 'dayjs';
import type { OverlayEventPayload, TaskLogEntry } from '@/api/soulbrowser';

import styles from './TasksPage.module.css';

interface ConversationBoardProps {
  planOverlays: Record<string, any>[];
  executionOverlays: OverlayEventPayload[];
  logs: TaskLogEntry[];
  recentEvidence: Record<string, any>[];
  onPreviewEvidence?: (item: Record<string, any>) => void;
  onPreviewOverlay?: (overlay: OverlayEventPayload) => void;
}

const MAX_LOGS = 5;
const MAX_OVERLAYS = 5;

const formatTime = (value?: string | null) =>
  value && dayjs(value).isValid() ? dayjs(value).format('HH:mm:ss') : '—';

const renderOverlayLabel = (overlay: Record<string, any>, fallback: string) => {
  if (overlay?.title) return overlay.title;
  if (overlay?.dispatch_label) return overlay.dispatch_label;
  if (overlay?.step_id) return `Step ${overlay.step_id}`;
  return fallback;
};

const overlayLevelColor = (level?: string) => {
  switch ((level || '').toLowerCase()) {
    case 'error':
      return 'red';
    case 'warn':
      return 'orange';
    case 'info':
      return 'geekblue';
    default:
      return 'purple';
  }
};

const isString = (value: unknown): value is string => typeof value === 'string';

export default function ConversationBoard({
  planOverlays,
  executionOverlays,
  logs,
  recentEvidence,
  onPreviewEvidence,
  onPreviewOverlay,
}: ConversationBoardProps) {
  const latestLogs = logs.slice(-MAX_LOGS).reverse();
  const latestOverlays = executionOverlays.slice(-MAX_OVERLAYS).reverse();
  const latestEvidence = recentEvidence.slice(-MAX_OVERLAYS).reverse();

  return (
    <div className={styles.conversationBoard}>
      <div className={styles.boardColumn}>
        <Typography.Title level={5}>计划锚点</Typography.Title>
        {planOverlays?.length ? (
          <List
            size="small"
            dataSource={planOverlays}
            renderItem={(item, index) => (
              <List.Item key={item?.step_id ?? index}>
                <Space direction="vertical" size={2}>
                  <Space size="small">
                    <Tag color="processing">{item?.step_id ?? `step-${index + 1}`}</Tag>
                    <Typography.Text>{renderOverlayLabel(item, '步骤')}</Typography.Text>
                  </Space>
                  <Typography.Text type="secondary" className={styles.locatorText}>
                    {formatTime(item?.recorded_at)}
                  </Typography.Text>
                  {item?.locator && (
                    <Typography.Text type="secondary" className={styles.locatorText}>
                      {JSON.stringify(item.locator)}
                    </Typography.Text>
                  )}
                </Space>
              </List.Item>
            )}
          />
        ) : (
          <Empty description="暂无锚点" />
        )}
      </div>

      <div className={styles.boardColumn}>
        <Typography.Title level={5}>实时覆盖</Typography.Title>
        {latestOverlays.length ? (
          <List
            size="small"
            dataSource={latestOverlays}
            renderItem={(item, index) => {
              const overlayData = (item.data ?? {}) as Record<string, unknown>;
              const overlayKind = isString(overlayData.kind) ? overlayData.kind : undefined;
              const overlayLevel = isString(overlayData.level) ? overlayData.level : undefined;
              const overlayDetail = isString(overlayData.detail) ? overlayData.detail : undefined;
              const overlayBase64 = isString(overlayData.data_base64)
                ? overlayData.data_base64
                : undefined;
              const overlayScreenshot = isString(overlayData.screenshot_path)
                ? overlayData.screenshot_path
                : undefined;
              const overlayContentType = isString(overlayData.content_type)
                ? overlayData.content_type
                : 'image/png';

              return (
                <List.Item key={`${item.task_id}-${index}`}>
                  <Space direction="vertical" size={2}>
                    <Space size="small" wrap>
                      <Tag color={item.source === 'execution' ? 'volcano' : 'geekblue'}>
                        {item.source === 'execution' ? '执行' : '计划'}
                      </Tag>
                      {overlayKind && (
                        <Tag color={overlayLevelColor(overlayLevel)}>{overlayKind}</Tag>
                      )}
                      <Typography.Text>
                        {renderOverlayLabel(overlayData as Record<string, any>, 'overlay')} ·
                        {overlayData.step_id || 'step'}
                      </Typography.Text>
                    </Space>
                    <Typography.Text type="secondary" className={styles.locatorText}>
                      {formatTime(item.recorded_at)}
                    </Typography.Text>
                    {overlayDetail && (
                      <Typography.Text type="secondary" className={styles.locatorText}>
                        {overlayDetail}
                      </Typography.Text>
                    )}
                    {overlayData.bbox !== undefined && (
                      <Typography.Text type="secondary" className={styles.locatorText}>
                        bbox: {JSON.stringify(overlayData.bbox)}
                      </Typography.Text>
                    )}
                    {overlayBase64 || overlayScreenshot ? (
                      <img
                        className={styles.overlayThumbnail}
                        src={
                          overlayBase64
                            ? `data:${overlayContentType};base64,${overlayBase64}`
                            : overlayScreenshot!
                        }
                        alt={renderOverlayLabel(overlayData as Record<string, any>, 'overlay')}
                        onClick={() => onPreviewOverlay?.(item)}
                      />
                    ) : null}
                  </Space>
                </List.Item>
              );
            }}
          />
        ) : (
          <Empty description="暂无实时覆盖" />
        )}

        <Typography.Title level={5} style={{ marginTop: 24 }}>
          最新截图
        </Typography.Title>
        {latestEvidence.length ? (
          <div className={styles.overlayPreviewGrid}>
            {latestEvidence.map((item, index) => {
              const evidenceLabel = isString(item?.label)
                ? item.label
                : isString(item?.dispatch_label)
                ? item.dispatch_label
                : '截图';
              const observationTag = isString(item?.observation_type)
                ? item.observation_type
                : isString(item?.content_type)
                ? item.content_type
                : undefined;
              const evidenceBase64 = isString(item.data_base64) ? item.data_base64 : undefined;
              const evidencePath = isString(item.screenshot_path) ? item.screenshot_path : undefined;
              const evidenceContentType = isString(item.content_type)
                ? item.content_type
                : 'image/png';

              return (
                <div className={styles.overlayPreviewCard} key={`${item?.step_id ?? 'shot'}-${index}`}>
                  <Typography.Text className={styles.evidenceLabel}>{evidenceLabel}</Typography.Text>
                  {observationTag && (
                    <Tag color="blue" style={{ marginBottom: 4 }}>
                      {observationTag}
                    </Tag>
                  )}
                  <Typography.Text type="secondary" className={styles.locatorText}>
                    {formatTime(item?.recorded_at)}
                  </Typography.Text>
                  {evidenceBase64 || evidencePath ? (
                    <img
                      src={
                        evidenceBase64
                          ? `data:${evidenceContentType};base64,${evidenceBase64}`
                          : evidencePath!
                      }
                      alt={evidenceLabel || `capture-${index}`}
                      className={styles.previewableImage}
                      onClick={() => onPreviewEvidence?.(item)}
                    />
                  ) : (
                    <Typography.Text type="secondary">暂无内嵌图像</Typography.Text>
                  )}
                </div>
              );
            })}
          </div>
        ) : (
          <Empty description="暂无截图" />
        )}
      </div>

      <div className={styles.boardColumn}>
        <Typography.Title level={5}>最新日志</Typography.Title>
        {latestLogs.length ? (
          <List
            size="small"
            dataSource={latestLogs}
            renderItem={(item, index) => (
              <List.Item key={`${item.timestamp}-${index}`}>
                <Space direction="vertical" size={0}>
                  <Typography.Text type="secondary">
                    {item.timestamp} · {item.level.toUpperCase()}
                  </Typography.Text>
                  <Typography.Text>{item.message}</Typography.Text>
                </Space>
              </List.Item>
            )}
          />
        ) : (
          <Empty description="暂无日志" />
        )}
      </div>
    </div>
  );
}
