import { useMemo, useRef, useState } from 'react';
import { Badge, Button, Card, Empty, List, Select, Space, Tag, Tooltip, Typography } from 'antd';
import {
  PauseCircleOutlined,
  PlayCircleOutlined,
  ReloadOutlined,
  ThunderboltOutlined,
} from '@ant-design/icons';
import dayjs from 'dayjs';
import type {
  AgentHistoryEntry,
  OverlayEventPayload,
  TaskAnnotation,
  TaskStatusSnapshot,
  WatchdogEvent,
  TaskJudgeVerdict,
  SelfHealEvent,
  TaskAlert,
} from '@/api/soulbrowser';
import VirtualList from 'rc-virtual-list';

import styles from './TasksPage.module.css';

export interface LiveFrame {
  id: string;
  recordedAt?: string;
  label?: string;
  src: string;
  record: Record<string, any>;
}

interface LiveCompanionProps {
  taskId?: string | null;
  status: TaskStatusSnapshot | null;
  latestFrame: LiveFrame | null;
  frames: LiveFrame[];
  overlays: OverlayEventPayload[];
  annotations: TaskAnnotation[];
  agentHistory: AgentHistoryEntry[];
  watchdogEvents: WatchdogEvent[];
  selfHealEvents: SelfHealEvent[];
  alerts: TaskAlert[];
  judgeVerdict: TaskJudgeVerdict | null;
  stepAnnotations: Record<string, TaskAnnotation[]>;
  annotationSeverityFilter: string;
  streamConnected: boolean;
  streamPaused: boolean;
  streamError: string | null;
  onTogglePause?: () => void;
  onReconnect?: () => void;
  onPreviewFrame?: (record: Record<string, any>) => void;
  onPreviewOverlay?: (overlay: OverlayEventPayload) => void;
  onPreviewStepEvidence?: (stepId: string) => void;
  hasEvidenceForStep?: (stepId?: string | null) => boolean;
  onAnnotationFilterChange?: (value: string) => void;
  onRefreshSummary?: () => void;
}

const formatTime = (value?: string | null) =>
  value && dayjs(value).isValid() ? dayjs(value).format('HH:mm:ss') : '—';

export default function LiveCompanion({
  taskId,
  status,
  latestFrame,
  frames,
  overlays,
  annotations,
  agentHistory,
  watchdogEvents,
  selfHealEvents,
  alerts,
  judgeVerdict,
  stepAnnotations,
  annotationSeverityFilter,
  streamConnected,
  streamPaused,
  streamError,
  onTogglePause,
  onReconnect,
  onPreviewFrame,
  onPreviewOverlay,
  onPreviewStepEvidence,
  hasEvidenceForStep,
  onAnnotationFilterChange,
  onRefreshSummary,
}: LiveCompanionProps) {
  const [imageSize, setImageSize] = useState({ width: 0, height: 0 });
  const [renderSize, setRenderSize] = useState({ width: 0, height: 0 });
  const imageRef = useRef<HTMLImageElement | null>(null);

  const overlayList = useMemo(() => overlays.slice(-6).reverse(), [overlays]);
  const filteredAnnotations = useMemo(() => {
    if (annotationSeverityFilter === 'all') {
      return annotations;
    }
    return annotations.filter(
      (annotation) => (annotation.severity || 'info') === annotationSeverityFilter
    );
  }, [annotationSeverityFilter, annotations]);
  const annotationList = useMemo(() => filteredAnnotations.slice(-6).reverse(), [filteredAnnotations]);
  const frameHistory = useMemo(() => frames.slice().reverse(), [frames]);
  const historyList = useMemo(() => agentHistory.slice().reverse(), [agentHistory]);
  const watchdogList = useMemo(() => watchdogEvents.slice(-6).reverse(), [watchdogEvents]);
  const selfHealList = useMemo(() => selfHealEvents.slice(-6).reverse(), [selfHealEvents]);
  const alertList = useMemo(() => alerts.slice(-6).reverse(), [alerts]);
  const filteredStepAnnotations = useMemo(() => {
    if (annotationSeverityFilter === 'all') {
      return stepAnnotations;
    }
    const map: Record<string, TaskAnnotation[]> = {};
    Object.entries(stepAnnotations).forEach(([stepId, entries]) => {
      const subset = entries.filter(
        (annotation) => (annotation.severity || 'info') === annotationSeverityFilter
      );
      if (subset.length) {
        map[stepId] = subset;
      }
    });
    return map;
  }, [annotationSeverityFilter, stepAnnotations]);

  const handleImageLoad: React.ReactEventHandler<HTMLImageElement> = (event) => {
    const target = event.currentTarget;
    setImageSize({ width: target.naturalWidth, height: target.naturalHeight });
    setRenderSize({ width: target.clientWidth, height: target.clientHeight });
    imageRef.current = target;
  };

  const computeBoxStyle = (bbox?: unknown) => {
    if (!bbox || typeof bbox !== 'object' || renderSize.width === 0 || renderSize.height === 0) {
      return undefined;
    }
    const parts = bbox as Record<string, any>;
    const width = Number(parts.width ?? parts.w ?? parts[2]);
    const height = Number(parts.height ?? parts.h ?? parts[3]);
    const x = Number(parts.x ?? parts.left ?? parts[0]);
    const y = Number(parts.y ?? parts.top ?? parts[1]);
    if (!Number.isFinite(width) || !Number.isFinite(height) || !Number.isFinite(x) || !Number.isFinite(y)) {
      return undefined;
    }
    const normalized =
      width > 0 && width <= 1 && height > 0 && height <= 1 && x >= 0 && x <= 1 && y >= 0 && y <= 1;
    const scaleX = normalized
      ? renderSize.width
      : imageSize.width > 0
      ? renderSize.width / imageSize.width
      : 1;
    const scaleY = normalized
      ? renderSize.height
      : imageSize.height > 0
      ? renderSize.height / imageSize.height
      : 1;
    const left = normalized ? x * renderSize.width : x * scaleX;
    const top = normalized ? y * renderSize.height : y * scaleY;
    const boxWidth = normalized ? width * renderSize.width : width * scaleX;
    const boxHeight = normalized ? height * renderSize.height : height * scaleY;
    return {
      left,
      top,
      width: Math.max(boxWidth, 2),
      height: Math.max(boxHeight, 2),
    } as React.CSSProperties;
  };

  return (
    <div className={styles.liveCompanion}>
      <div className={styles.companionHeader}>
        <Space size="middle" wrap>
          <Badge
            status={streamConnected ? 'processing' : 'default'}
            text={streamConnected ? '实时连接中' : streamPaused ? '已暂停' : '未连接'}
          />
          <Tag color={status?.status === 'success' ? 'green' : status?.status === 'failed' ? 'red' : 'gold'}>
            {status?.status?.toUpperCase() || 'PENDING'}
          </Tag>
          {latestFrame?.recordedAt && (
            <Typography.Text type="secondary">
              最新截图：{formatTime(latestFrame.recordedAt)}
            </Typography.Text>
          )}
         {taskId && (
            <Typography.Text type="secondary" copyable>
              {taskId}
            </Typography.Text>
          )}
          {judgeVerdict && (
            <Tag color={judgeVerdict.verdict.passed ? 'green' : 'red'}>
              QA {judgeVerdict.verdict.passed ? '通过' : '未通过'}
            </Tag>
          )}
          {judgeVerdict?.verdict.reason && (
            <Typography.Text type="secondary" style={{ fontSize: 12 }}>
              {judgeVerdict.verdict.reason}
            </Typography.Text>
          )}
          {streamError && (
            <Tooltip title={streamError}>
              <Tag color="red">{streamError}</Tag>
            </Tooltip>
          )}
        </Space>
        <Space>
          <Button
            icon={streamPaused ? <PlayCircleOutlined /> : <PauseCircleOutlined />}
            onClick={onTogglePause}
          >
            {streamPaused ? '恢复实时' : '暂停实时'}
          </Button>
          <Button icon={<ThunderboltOutlined />} onClick={onReconnect}>
            手动重连
          </Button>
          <Button icon={<ReloadOutlined />} onClick={onRefreshSummary}>
            刷新快照
          </Button>
        </Space>
      </div>

      <div className={styles.liveBody}>
        <div className={styles.liveViewport}>
          {latestFrame?.src ? (
            <div className={styles.liveOverlayStage}>
              <img
                ref={imageRef}
                src={latestFrame.src}
                alt={latestFrame.label || 'live-frame'}
                className={styles.liveScreenshot}
                onLoad={handleImageLoad}
                onClick={() => onPreviewFrame?.(latestFrame.record)}
              />
              {overlayList.map((overlay, index) => {
                const style = computeBoxStyle(overlay.data?.bbox);
                if (!style) {
                  return null;
                }
                const overlayData = overlay.data as Record<string, unknown> | undefined;
                const rawLabel = overlayData?.title ?? overlayData?.dispatch_label ?? 'overlay';
                const label =
                  typeof rawLabel === 'string'
                    ? rawLabel
                    : JSON.stringify(rawLabel ?? 'overlay');
                return (
                  <div
                    key={`${overlay.task_id}-${index}`}
                    className={styles.liveOverlayBox}
                    style={style}
                  >
                    <span>{label}</span>
                  </div>
                );
              })}
            </div>
          ) : (
            <div className={styles.liveViewportFallback}>
              <Empty description="暂无实时截图" />
            </div>
          )}
        </div>

        <div className={styles.liveSidebar}>
          <Card size="small" title="步骤进度">
            {historyList.length ? (
              <div className={styles.liveVirtualList}>
                <VirtualList
                  data={historyList}
                  height={280}
                  itemHeight={80}
                  itemKey={(entry) => `${entry.step_id}-${entry.timestamp}`}
                >
                  {(entry) => {
                    const annotationsForStep = entry.step_id
                      ? filteredStepAnnotations[entry.step_id] ?? []
                      : [];
                    const latestNote =
                      annotationsForStep.length > 0
                        ? annotationsForStep[annotationsForStep.length - 1]
                        : null;
                    return (
                      <List.Item key={`${entry.step_id}-${entry.timestamp}`} className={styles.liveListItem}>
                        <Space direction="vertical" size={0} style={{ width: '100%' }}>
                          <Space size="small" wrap>
                            <Tag color={entry.status === 'success' ? 'green' : 'red'}>
                              {entry.status === 'success' ? '成功' : '失败'}
                            </Tag>
                            <Typography.Text>{entry.title || `步骤 ${entry.step_index + 1}`}</Typography.Text>
                            <Typography.Text type="secondary" style={{ fontSize: 12 }}>
                              {formatTime(entry.timestamp)}
                            </Typography.Text>
                            {annotationsForStep.length > 0 && (
                              <Tag color="magenta">批注 {annotationsForStep.length}</Tag>
                            )}
                            {entry.obstruction && <Tag color="volcano">{entry.obstruction}</Tag>}
                          </Space>
                          {(entry.observation_summary || entry.structured_summary) && (
                            <Typography.Text type="secondary" style={{ fontSize: 12 }}>
                              {entry.observation_summary || entry.structured_summary}
                            </Typography.Text>
                          )}
                          {latestNote && (
                            <Typography.Text type="secondary" style={{ fontSize: 12 }}>
                              最近批注：{latestNote.note}
                            </Typography.Text>
                          )}
                          <Space size="small">
                            <Typography.Text type="secondary" style={{ fontSize: 12 }}>
                              尝试 {entry.attempts}
                            </Typography.Text>
                            <Button
                              type="link"
                              size="small"
                              disabled={!entry.step_id || !hasEvidenceForStep?.(entry.step_id)}
                              onClick={() => entry.step_id && onPreviewStepEvidence?.(entry.step_id)}
                            >
                              查看截图
                            </Button>
                          </Space>
                        </Space>
                      </List.Item>
                    );
                  }}
                </VirtualList>
              </div>
            ) : (
              <Empty description="暂无进度" image={Empty.PRESENTED_IMAGE_SIMPLE} />
            )}
          </Card>

          <Card size="small" title="实时覆盖">
            {overlayList.length ? (
              <List
                size="small"
                dataSource={overlayList}
                renderItem={(item, index) => (
                  <List.Item
                    key={`${item.task_id}-${index}`}
                    onClick={() => onPreviewOverlay?.(item)}
                    className={styles.liveListItem}
                  >
                    <Space direction="vertical" size={0}>
                      <Space size="small">
                        <Tag color={item.source === 'execution' ? 'orange' : 'blue'}>{item.source}</Tag>
                        <Typography.Text>
                          {(() => {
                            const data = item.data as Record<string, unknown> | undefined;
                            const candidate = data?.title ?? data?.dispatch_label ?? 'overlay';
                            return typeof candidate === 'string'
                              ? candidate
                              : JSON.stringify(candidate ?? 'overlay');
                          })()}
                        </Typography.Text>
                      </Space>
                      <Typography.Text type="secondary" style={{ fontSize: 12 }}>
                        {formatTime(item.recorded_at)}
                      </Typography.Text>
                    </Space>
                  </List.Item>
                )}
              />
            ) : (
              <Empty description="暂无 Overlay" image={Empty.PRESENTED_IMAGE_SIMPLE} />
            )}
          </Card>

          <Card
            size="small"
            title={
              <Space>
                <span>批注</span>
                <Select
                  size="small"
                  value={annotationSeverityFilter}
                  style={{ width: 120 }}
                  onChange={onAnnotationFilterChange}
                  options={[
                    { value: 'all', label: '全部' },
                    { value: 'info', label: 'info' },
                    { value: 'warn', label: 'warn' },
                    { value: 'critical', label: 'critical' },
                  ]}
                />
              </Space>
            }
          >
            {annotationList.length ? (
              <List
                size="small"
                dataSource={annotationList}
                renderItem={(annotation) => (
                  <List.Item key={annotation.id} className={styles.liveListItem}>
                    <Space direction="vertical" size={0}>
                      <Space size="small">
                        {annotation.severity && <Tag color="magenta">{annotation.severity}</Tag>}
                        {annotation.kind && <Tag color="cyan">{annotation.kind}</Tag>}
                        <Typography.Text>{annotation.note}</Typography.Text>
                      </Space>
                      <Typography.Text type="secondary" style={{ fontSize: 12 }}>
                        {annotation.author || 'system'} · {formatTime(annotation.created_at)}
                      </Typography.Text>
                    </Space>
                  </List.Item>
                )}
              />
            ) : (
              <Empty description="暂无批注" image={Empty.PRESENTED_IMAGE_SIMPLE} />
            )}
          </Card>

          <Card size="small" title="Watchdog 事件">
            {watchdogList.length ? (
              <List
                size="small"
                dataSource={watchdogList}
                renderItem={(event) => (
                  <List.Item key={event.id} className={styles.liveListItem}>
                    <Space direction="vertical" size={0}>
                      <Space size="small">
                        <Tag color={event.severity === 'critical' ? 'red' : event.severity === 'warn' ? 'orange' : 'blue'}>
                          {event.severity}
                        </Tag>
                        <Tag>{event.kind}</Tag>
                        <Typography.Text>{event.note}</Typography.Text>
                      </Space>
                      <Typography.Text type="secondary" style={{ fontSize: 12 }}>
                        {formatTime(event.recorded_at)}
                      </Typography.Text>
                    </Space>
                  </List.Item>
                )}
              />
            ) : (
              <Empty description="暂无 Watchdog 事件" image={Empty.PRESENTED_IMAGE_SIMPLE} />
            )}
          </Card>

          <Card size="small" title="告警">
            {alertList.length ? (
              <List
                size="small"
                dataSource={alertList}
                renderItem={(alert) => (
                  <List.Item key={`${alert.kind ?? 'alert'}-${alert.timestamp}`} className={styles.liveListItem}>
                    <Space direction="vertical" size={0}>
                      <Space size="small">
                        <Tag color={alert.severity === 'critical' ? 'red' : alert.severity === 'warn' ? 'orange' : 'blue'}>
                          {alert.severity}
                        </Tag>
                        {alert.kind && <Tag>{alert.kind}</Tag>}
                        <Typography.Text>{alert.message}</Typography.Text>
                      </Space>
                      <Typography.Text type="secondary" style={{ fontSize: 12 }}>
                        {formatTime(alert.timestamp)}
                      </Typography.Text>
                    </Space>
                  </List.Item>
                )}
              />
            ) : (
              <Empty description="暂无告警" image={Empty.PRESENTED_IMAGE_SIMPLE} />
            )}
          </Card>

          <Card size="small" title="自愈策略">
            {selfHealList.length ? (
              <List
                size="small"
                dataSource={selfHealList}
                renderItem={(event) => (
                  <List.Item key={`${event.strategy_id}-${event.timestamp}`} className={styles.liveListItem}>
                    <Space direction="vertical" size={0}>
                      <Space size="small">
                        <Tag color="geekblue">{event.strategy_id}</Tag>
                        <Typography.Text>{event.action}</Typography.Text>
                      </Space>
                      {event.note && (
                        <Typography.Text type="secondary" style={{ fontSize: 12 }}>
                          {event.note}
                        </Typography.Text>
                      )}
                      <Typography.Text type="secondary" style={{ fontSize: 12 }}>
                        {formatTime(new Date(event.timestamp).toISOString())}
                      </Typography.Text>
                    </Space>
                  </List.Item>
                )}
              />
            ) : (
              <Empty description="暂无自愈事件" image={Empty.PRESENTED_IMAGE_SIMPLE} />
            )}
          </Card>

          <Card size="small" title="截图历史">
            {frameHistory.length ? (
              <List
                size="small"
                className={styles.liveFrameList}
                dataSource={frameHistory}
                renderItem={(frame) => (
                  <List.Item
                    key={frame.id}
                    className={styles.liveListItem}
                    onClick={() => onPreviewFrame?.(frame.record)}
                  >
                    <Space direction="vertical" size={0}>
                      <Typography.Text>{frame.label || 'capture'}</Typography.Text>
                      <Typography.Text type="secondary" style={{ fontSize: 12 }}>
                        {formatTime(frame.recordedAt)}
                      </Typography.Text>
                    </Space>
                  </List.Item>
                )}
              />
            ) : (
              <Empty description="暂无截图" image={Empty.PRESENTED_IMAGE_SIMPLE} />
            )}
          </Card>
        </div>
      </div>
    </div>
  );
}
