import { useEffect, useMemo, useState } from 'react';
import {
  Card,
  Input,
  InputNumber,
  Button,
  Space,
  Alert,
  Typography,
  Select,
  message,
  Descriptions,
  List,
  Spin,
  Switch,
  Tag,
  Statistic,
  Form,
  Modal,
  Divider,
} from 'antd';
import {
  ApiOutlined,
  AppstoreAddOutlined,
  LinkOutlined,
  HeartOutlined,
  RobotOutlined,
  EyeOutlined,
  DatabaseOutlined,
  DownloadOutlined,
  SafetyCertificateOutlined,
  MinusCircleOutlined,
  PlusOutlined,
  SearchOutlined,
} from '@ant-design/icons';
import soulbrowserAPI, {
  type PluginRegistryRecord,
  type PluginRegistryStats,
  type MemoryStatsSnapshot,
  type PerceiveResponse,
  type PerceptionMetrics,
  type RecordingSummary,
  type RegistryHelper,
  type RegistryHelperStep,
  type SelfHealAction,
  type SelfHealStrategy,
} from '@/api/soulbrowser';
import { useBackendConfigStore } from '@/stores/backendConfigStore';
import styles from './DiagnosticsPage.module.css';

const { TextArea } = Input;
const { Paragraph } = Typography;

type HealthState = 'idle' | 'loading' | 'success' | 'error';

type PerceiveMode = 'all' | 'structural' | 'visual' | 'semantic';

const DEFAULT_CHAT_PROMPT = 'Navigate to https://example.com and tell me what you see';

interface HelperStepFormValues {
  title: string;
  detail?: string;
  wait?: string;
  timeout_ms?: number;
  tool_type: 'click_css' | 'click_text' | 'custom';
  selector?: string;
  text?: string;
  exact?: boolean;
  name?: string;
  payload?: string;
}

interface HelperFormValues {
  id: string;
  pattern: string;
  description?: string;
  prompt?: string;
  auto_insert: boolean;
  blockers?: string[];
  url_includes?: string[];
  url_excludes?: string[];
  steps: HelperStepFormValues[];
}

export default function DiagnosticsPage() {
  const { baseUrl, setBaseUrl } = useBackendConfigStore();
  const [editingBaseUrl, setEditingBaseUrl] = useState(baseUrl);

  const [healthState, setHealthState] = useState<HealthState>('idle');
  const [healthResult, setHealthResult] = useState('');

  const [chatPrompt, setChatPrompt] = useState(DEFAULT_CHAT_PROMPT);
  const [chatLoading, setChatLoading] = useState(false);
  const [chatError, setChatError] = useState<string | null>(null);
  const [chatOutput, setChatOutput] = useState<any>(null);

  const [perceiveUrl, setPerceiveUrl] = useState('https://example.com');
  const [perceiveMode, setPerceiveMode] = useState<PerceiveMode>('structural');
  const [perceiveTimeout, setPerceiveTimeout] = useState(90);
  const [perceiveLoading, setPerceiveLoading] = useState(false);
  const [perceiveError, setPerceiveError] = useState<string | null>(null);
  const [perceiveOutput, setPerceiveOutput] = useState<PerceiveResponse | null>(null);
  const [metrics, setMetrics] = useState<PerceptionMetrics | null>(null);
  const [metricsLoading, setMetricsLoading] = useState(false);
  const [recordings, setRecordings] = useState<RecordingSummary[]>([]);
  const [recordingsLoading, setRecordingsLoading] = useState(false);
  const [recordingsLimit, setRecordingsLimit] = useState(10);
  const [recordingsStateFilter, setRecordingsStateFilter] = useState<string | undefined>();
  const [recordingDetail, setRecordingDetail] = useState<any>(null);
  const [recordingDetailLoading, setRecordingDetailLoading] = useState(false);
  const [selfHealStrategies, setSelfHealStrategies] = useState<SelfHealStrategy[]>([]);
  const [selfHealLoading, setSelfHealLoading] = useState(false);
  const [memoryStats, setMemoryStats] = useState<MemoryStatsSnapshot | null>(null);
  const [memoryStatsLoading, setMemoryStatsLoading] = useState(false);
  const [pluginRegistry, setPluginRegistry] = useState<PluginRegistryRecord[]>([]);
  const [pluginStats, setPluginStats] = useState<PluginRegistryStats | null>(null);
  const [pluginLoading, setPluginLoading] = useState(false);
  const [pluginStatusFilter, setPluginStatusFilter] = useState<string | undefined>();
  const [pluginSearch, setPluginSearch] = useState('');
  const [helperModalVisible, setHelperModalVisible] = useState(false);
  const [helperModalPlugin, setHelperModalPlugin] = useState<string | null>(null);
  const [editingHelper, setEditingHelper] = useState<RegistryHelper | null>(null);
  const [helperSaving, setHelperSaving] = useState(false);
  const [helperForm] = Form.useForm<HelperFormValues>();
  const [helperPreview, setHelperPreview] = useState<string>('');
  const [helperPreviewError, setHelperPreviewError] = useState<string | null>(null);

  const handleSaveBaseUrl = () => {
    setBaseUrl(editingBaseUrl);
    message.success('后端地址已更新');
  };

  const runHealthCheck = async () => {
    setHealthState('loading');
    setHealthResult('');
    try {
      const response = await soulbrowserAPI.health();
      setHealthState('success');
      setHealthResult(JSON.stringify(response, null, 2));
    } catch (err) {
      setHealthState('error');
      setHealthResult(err instanceof Error ? err.message : 'Health check failed');
    }
  };

  const runChatTest = async () => {
    if (!chatPrompt.trim()) {
      message.warning('请输入测试提示词');
      return;
    }

    setChatLoading(true);
    setChatError(null);
    setChatOutput(null);

    try {
      const response = await soulbrowserAPI.chat({ prompt: chatPrompt, execute: false });
      setChatOutput(response);
    } catch (err) {
      setChatError(err instanceof Error ? err.message : 'Chat API 调用失败');
    } finally {
      setChatLoading(false);
    }
  };

  const runPerceiveTest = async () => {
    if (!perceiveUrl.trim()) {
      message.warning('请输入要感知的 URL');
      return;
    }

    setPerceiveLoading(true);
    setPerceiveError(null);
    setPerceiveOutput(null);

    try {
      const response = await soulbrowserAPI.perceive({
        url: perceiveUrl,
        mode: perceiveMode,
        screenshot: false,
        insights: perceiveMode === 'all',
        timeout: perceiveTimeout,
      });
      setPerceiveOutput(response);
      if (!response.success) {
        setPerceiveError(response.error || 'Perceive API 返回失败');
      }
    } catch (err) {
      setPerceiveError(err instanceof Error ? err.message : 'Perceive API 调用失败');
    } finally {
      setPerceiveLoading(false);
    }
  };

  const renderResultBlock = (data: unknown) => (
    <pre className={styles.resultBlock}>{JSON.stringify(data, null, 2)}</pre>
  );

  const loadPerceptionMetrics = async () => {
    setMetricsLoading(true);
    try {
      const snapshot = await soulbrowserAPI.getPerceptionMetrics();
      setMetrics(snapshot);
    } catch (err) {
      message.error('无法获取感知指标');
    } finally {
      setMetricsLoading(false);
    }
  };

  useEffect(() => {
    void loadPerceptionMetrics();
  }, []);

  useEffect(() => {
    void loadSelfHealStrategies();
  }, []);

  useEffect(() => {
    void loadMemoryStats();
  }, []);

  useEffect(() => {
    void loadPluginRegistry();
  }, []);

  useEffect(() => {
    void loadRecordings();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [recordingsLimit, recordingsStateFilter]);

  const loadRecordings = async () => {
    setRecordingsLoading(true);
    try {
      const response = await soulbrowserAPI.listRecordings(recordingsLimit, recordingsStateFilter);
      setRecordings(response.recordings || []);
    } catch (err) {
      console.error(err);
      message.error('无法获取录制会话');
    } finally {
      setRecordingsLoading(false);
    }
  };

  const loadRecordingDetail = async (sessionId: string) => {
    setRecordingDetailLoading(true);
    setRecordingDetail(null);
    try {
      const response = await soulbrowserAPI.getRecording(sessionId);
      if (response.success) {
        setRecordingDetail(response.recording);
      } else {
        message.warning('录制会话未找到');
      }
    } catch (err) {
      console.error(err);
      message.error('获取录制详情失败');
    } finally {
      setRecordingDetailLoading(false);
    }
  };

  const loadSelfHealStrategies = async () => {
    setSelfHealLoading(true);
    try {
      const strategies = await soulbrowserAPI.listSelfHealStrategies();
      setSelfHealStrategies(strategies);
    } catch (err) {
      console.error(err);
      message.error('无法获取自愈策略');
    } finally {
      setSelfHealLoading(false);
    }
  };

  const loadMemoryStats = async () => {
    setMemoryStatsLoading(true);
    try {
      const snapshot = await soulbrowserAPI.getMemoryStats();
      setMemoryStats(snapshot);
    } catch (err) {
      console.error(err);
      message.error('无法获取 Memory 统计');
    } finally {
      setMemoryStatsLoading(false);
    }
  };

  const loadPluginRegistry = async () => {
    setPluginLoading(true);
    try {
      const response = await soulbrowserAPI.listPluginRegistry();
      setPluginStats(response.stats);
      setPluginRegistry(response.plugins);
    } catch (err) {
      console.error(err);
      message.error('无法获取插件注册表');
    } finally {
      setPluginLoading(false);
    }
  };

  const filteredPlugins = useMemo(() => {
    return pluginRegistry.filter((plugin) => {
      const statusMatches = pluginStatusFilter
        ? (plugin.status || 'pending').toLowerCase() === pluginStatusFilter
        : true;
      if (!statusMatches) {
        return false;
      }
      if (!pluginSearch.trim()) {
        return true;
      }
      const needle = pluginSearch.toLowerCase();
      return (
        plugin.id.toLowerCase().includes(needle) ||
        plugin.description?.toLowerCase().includes(needle) ||
        plugin.owner?.toLowerCase().includes(needle) ||
        plugin.helpers?.some((helper) =>
          helper.id.toLowerCase().includes(needle) ||
          helper.description?.toLowerCase().includes(needle)
        )
      );
    });
  }, [pluginRegistry, pluginStatusFilter, pluginSearch]);

  const handleStrategyToggle = async (strategy: SelfHealStrategy, enabled: boolean) => {
    setSelfHealStrategies((current) =>
      current.map((item) => (item.id === strategy.id ? { ...item, enabled } : item))
    );
    try {
      await soulbrowserAPI.setSelfHealStrategyEnabled(strategy.id, enabled);
      message.success(`${strategy.id} ${enabled ? '已启用' : '已关闭'}`);
    } catch (err) {
      setSelfHealStrategies((current) =>
        current.map((item) =>
          item.id === strategy.id ? { ...item, enabled: strategy.enabled } : item
        )
      );
      message.error(err instanceof Error ? err.message : '更新策略失败');
    }
  };

  const defaultHelperStep = (): HelperStepFormValues => ({
    title: '',
    detail: '',
    wait: 'dom_ready',
    timeout_ms: undefined,
    tool_type: 'click_css',
    selector: '',
  });

  const buildDefaultHelperValues = (): HelperFormValues => ({
    id: '',
    pattern: '',
    description: '',
    prompt: '',
    auto_insert: true,
    blockers: [],
    url_includes: [],
    url_excludes: [],
    steps: [defaultHelperStep()],
  });

  const helperToFormValues = (helper: RegistryHelper): HelperFormValues => {
    const steps = helper.steps && helper.steps.length ? helper.steps : [];
    const resolvedSteps = steps.length
      ? steps
      : helper.step
        ? [helper.step]
        : [];
    return {
      id: helper.id,
      pattern: helper.pattern,
      description: helper.description ?? undefined,
      prompt: helper.prompt ?? undefined,
      auto_insert: helper.auto_insert ?? false,
      blockers: helper.blockers ?? [],
      url_includes: helper.conditions?.url_includes ?? [],
      url_excludes: helper.conditions?.url_excludes ?? [],
      steps: resolvedSteps.length
        ? resolvedSteps.map((step: RegistryHelperStep) => {
            const base: HelperStepFormValues = {
              title: step.title,
              detail: step.detail ?? undefined,
              wait: step.wait ?? undefined,
              timeout_ms: step.timeout_ms ?? undefined,
              tool_type: 'click_css',
            };
            switch (step.tool.type) {
              case 'click_text':
                return {
                  ...base,
                  tool_type: 'click_text',
                  text: step.tool.text,
                  exact: step.tool.exact ?? false,
                };
              case 'custom':
                return {
                  ...base,
                  tool_type: 'custom',
                  name: step.tool.name,
                  payload: step.tool.payload
                    ? JSON.stringify(step.tool.payload, null, 2)
                    : '',
                };
              default:
                return {
                  ...base,
                  tool_type: 'click_css',
                  selector: step.tool.selector,
                };
            }
          })
        : [defaultHelperStep()],
    };
  };

  const helperFormToPayload = (values: HelperFormValues): RegistryHelper => {
    if (!values.steps || values.steps.length === 0) {
      throw new Error('必须至少配置一个步骤');
    }
    const steps = values.steps.map((step, index) => {
      if (!step.title?.trim()) {
        throw new Error(`步骤 ${index + 1} 需要标题`);
      }
      switch (step.tool_type) {
        case 'click_css':
          if (!step.selector?.trim()) {
            throw new Error(`步骤 ${index + 1} 需要 CSS 选择器`);
          }
          return {
            title: step.title.trim(),
            detail: step.detail?.trim() || undefined,
            wait: step.wait || undefined,
            timeout_ms: step.timeout_ms,
            tool: { type: 'click_css', selector: step.selector.trim() },
          };
        case 'click_text':
          if (!step.text?.trim()) {
            throw new Error(`步骤 ${index + 1} 需要文本内容`);
          }
          return {
            title: step.title.trim(),
            detail: step.detail?.trim() || undefined,
            wait: step.wait || undefined,
            timeout_ms: step.timeout_ms,
            tool: {
              type: 'click_text',
              text: step.text.trim(),
              exact: step.exact ?? false,
            },
          };
        case 'custom': {
          if (!step.name?.trim()) {
            throw new Error(`步骤 ${index + 1} 需要自定义工具名称`);
          }
          let payload: Record<string, unknown> | undefined;
          if (step.payload && step.payload.trim().length > 0) {
            try {
              payload = JSON.parse(step.payload);
            } catch (err) {
              throw new Error(`步骤 ${index + 1} 的自定义 Payload 不是有效 JSON`);
            }
          }
          return {
            title: step.title.trim(),
            detail: step.detail?.trim() || undefined,
            wait: step.wait || undefined,
            timeout_ms: step.timeout_ms,
            tool: {
              type: 'custom',
              name: step.name.trim(),
              payload,
            },
          };
        }
        default:
          throw new Error(`未知的工具类型: ${step.tool_type}`);
      }
    });

    return {
      id: values.id.trim(),
      pattern: values.pattern.trim(),
      description: values.description?.trim() || undefined,
      prompt: values.prompt?.trim() || undefined,
      auto_insert: values.auto_insert,
      blockers: values.blockers?.filter((b) => b.trim().length > 0) ?? [],
      steps,
      conditions: {
        url_includes: values.url_includes?.filter((v) => v.trim().length > 0) ?? [],
        url_excludes: values.url_excludes?.filter((v) => v.trim().length > 0) ?? [],
      },
    } as RegistryHelper;
  };

  const handleHelperFormChange = async () => {
    try {
      const values = helperForm.getFieldsValue(true);
      const payload = helperFormToPayload(values);
      setHelperPreview(JSON.stringify(payload, null, 2));
      setHelperPreviewError(null);
    } catch (err) {
      setHelperPreview('');
      if (err instanceof Error) {
        setHelperPreviewError(err.message);
      } else {
        setHelperPreviewError('表单校验失败');
      }
    }
  };

  const openCreateHelperModal = (pluginId: string) => {
    setHelperModalPlugin(pluginId);
    setEditingHelper(null);
    helperForm.setFieldsValue(buildDefaultHelperValues());
    setHelperPreview('');
    setHelperPreviewError(null);
    setHelperModalVisible(true);
  };

  const openEditHelperModal = (pluginId: string, helper: RegistryHelper) => {
    setHelperModalPlugin(pluginId);
    setEditingHelper(helper);
    helperForm.setFieldsValue(helperToFormValues(helper));
    setHelperPreview(JSON.stringify(helper, null, 2));
    setHelperPreviewError(null);
    setHelperModalVisible(true);
  };

  const closeHelperModal = () => {
    setHelperModalVisible(false);
    setEditingHelper(null);
    setHelperModalPlugin(null);
  };

  const handleHelperSubmit = async () => {
    try {
      const values = await helperForm.validateFields();
      const payload = helperFormToPayload(values);
      if (!helperModalPlugin) {
        throw new Error('未选择插件');
      }
      setHelperSaving(true);
      if (editingHelper) {
        await soulbrowserAPI.updatePluginHelper(helperModalPlugin, editingHelper.id, payload);
        message.success(`Helper ${editingHelper.id} 已更新`);
      } else {
        await soulbrowserAPI.createPluginHelper(helperModalPlugin, payload);
        message.success('Helper 已创建');
      }
      closeHelperModal();
      await loadPluginRegistry();
    } catch (err) {
      if (err instanceof Error) {
        message.error(err.message);
      }
    } finally {
      setHelperSaving(false);
    }
  };

  const handleHelperDelete = (pluginId: string, helperId: string) => {
    Modal.confirm({
      title: `删除 Helper ${helperId}?`,
      content: '此操作不可恢复。',
      okText: '删除',
      okType: 'danger',
      cancelText: '取消',
      onOk: async () => {
        try {
          await soulbrowserAPI.deletePluginHelper(pluginId, helperId);
          message.success('Helper 已删除');
          await loadPluginRegistry();
        } catch (err) {
          message.error(err instanceof Error ? err.message : '删除失败');
        }
      },
    });
  };

  const handlePluginStatusChange = async (
    plugin: PluginRegistryRecord,
    status: 'active' | 'pending' | 'disabled'
  ) => {
    setPluginRegistry((current) =>
      current.map((item) => (item.id === plugin.id ? { ...item, status } : item))
    );
    try {
      await soulbrowserAPI.updatePluginStatus(plugin.id, status);
      message.success(`${plugin.id} 状态已更新`);
      void loadPluginRegistry();
    } catch (err) {
      console.error(err);
      message.error(err instanceof Error ? err.message : '更新插件状态失败');
      void loadPluginRegistry();
    }
  };

  const actionTagColor = (kind: SelfHealAction['kind']) => {
    switch (kind) {
      case 'auto_retry':
        return 'green';
      case 'human_approval':
        return 'magenta';
      default:
        return 'blue';
    }
  };

  const describeAction = (strategy: SelfHealStrategy) => {
    switch (strategy.action.kind) {
      case 'auto_retry':
        return `额外重试 ${strategy.action.extra_attempts} 次`;
      case 'annotate':
        return `标注 ${strategy.action.severity ?? 'info'}${
          strategy.action.note ? ` · ${strategy.action.note}` : ''
        }`;
      case 'human_approval':
        return `人工确认 (${strategy.action.severity ?? 'warn'})`;
      default:
        return '';
    }
  };

  const formatPercent = (value?: number) =>
    typeof value === 'number' ? `${(value * 100).toFixed(1)}%` : '—';

  const memoryAlerts: { type: 'warning' | 'error'; message: string; description?: string }[] = [];
  if (memoryStats) {
    if (memoryStats.hit_rate < 0.5) {
      memoryAlerts.push({
        type: 'warning',
        message: 'Memory 命中率偏低',
        description: `当前命中率 ${formatPercent(memoryStats.hit_rate)}，请检查 namespace/tag 配置。`,
      });
    }
    if (memoryStats.template_success_rate < 0.5 && memoryStats.template_uses > 10) {
      memoryAlerts.push({
        type: 'warning',
        message: '模板成功率偏低',
        description: `模板成功率 ${formatPercent(memoryStats.template_success_rate)}，请复核模板内容。`,
      });
    }
  }

  const handleDownloadRecordingPlan = (recording: any) => {
    const plan = recording.metadata?.agent_plan;
    if (!plan) {
      message.info('该录制会话没有计划快照');
      return;
    }
    try {
      const blob = new Blob([JSON.stringify(plan, null, 2)], { type: 'application/json' });
      const url = URL.createObjectURL(blob);
      const anchor = document.createElement('a');
      anchor.href = url;
      anchor.download = `${recording.id}-plan.json`;
      anchor.click();
      URL.revokeObjectURL(url);
    } catch (err) {
      console.error(err);
      message.error('导出计划失败');
    }
  };

  return (
    <div className={styles.wrapper}>
      <Card
        className={styles.sectionCard}
        title={
          <div className={styles.cardTitle}>
            <LinkOutlined className={styles.cardTitleIcon} />
            连接设置
          </div>
        }
      >
        <Space direction="vertical" style={{ width: '100%' }} size="large">
          <div className={styles.inlineForm}>
            <Input
              placeholder="http://127.0.0.1:8791"
              value={editingBaseUrl}
              onChange={(e) => setEditingBaseUrl(e.target.value)}
              allowClear
            />
            <Button type="primary" icon={<ApiOutlined />} onClick={handleSaveBaseUrl}>
              保存并应用
            </Button>
          </div>
          <Paragraph className={styles.hintText}>
            不填表示使用当前域名。若通过 Vite DevServer 调试，可填写实际后端地址 (示例: http://127.0.0.1:8791)。
          </Paragraph>
        </Space>
      </Card>

      <Card
        className={styles.sectionCard}
        title={
          <div className={styles.cardTitle}>
            <DatabaseOutlined className={styles.cardTitleIcon} />
            Memory 统计
          </div>
        }
        extra={
          <Button type="link" onClick={() => loadMemoryStats()} loading={memoryStatsLoading}>
            刷新
          </Button>
        }
      >
        {memoryStatsLoading ? (
          <Spin />
        ) : memoryStats ? (
          <Space direction="vertical" style={{ width: '100%' }} size="middle">
            {memoryAlerts.map((alert, idx) => (
              <Alert
                key={`memory-alert-${idx}`}
                type={alert.type}
                showIcon
                message={alert.message}
                description={alert.description}
              />
            ))}
            <div className={styles.memoryStatsGrid}>
              <Statistic title="总查询" value={memoryStats.total_queries} />
              <Statistic title="命中" value={memoryStats.hit_queries} />
              <Statistic title="未命中" value={memoryStats.miss_queries} />
              <Statistic
                title="命中率"
                value={formatPercent(memoryStats.hit_rate)}
                valueStyle={{ color: memoryStats.hit_rate >= 0.7 ? '#3f8600' : '#faad14' }}
              />
              <Statistic title="当前记录" value={memoryStats.current_records} />
              <Statistic title="累计写入" value={memoryStats.stored_records} />
              <Statistic title="删除" value={memoryStats.deleted_records} />
              <Statistic title="模板使用" value={memoryStats.template_uses} />
              <Statistic title="模板成功" value={memoryStats.template_successes} />
              <Statistic
                title="模板成功率"
                value={formatPercent(memoryStats.template_success_rate)}
                valueStyle={{
                  color: memoryStats.template_success_rate >= 0.7 ? '#3f8600' : '#faad14',
                }}
              />
            </div>
          </Space>
        ) : (
          <Paragraph className={styles.hintText}>
            暂无统计信息。执行一次 Memory 查询后刷新以查看最新数据。
          </Paragraph>
        )}
      </Card>

      <Card
        className={styles.sectionCard}
        title={
          <div className={styles.cardTitle}>
            <SafetyCertificateOutlined className={styles.cardTitleIcon} />
            自愈策略
          </div>
        }
        extra={
          <Button type="link" onClick={() => loadSelfHealStrategies()} loading={selfHealLoading}>
            刷新
          </Button>
        }
      >
        {selfHealLoading ? (
          <Spin />
        ) : selfHealStrategies.length ? (
          <List
            itemLayout="vertical"
            dataSource={selfHealStrategies}
            renderItem={(item) => (
              <List.Item
                key={item.id}
                className={styles.strategyRow}
                actions={[
                  <Switch
                    key="toggle"
                    checked={item.enabled}
                    size="small"
                    onChange={(checked) => handleStrategyToggle(item, checked)}
                  />,
                ]}
              >
                <List.Item.Meta
                  title={
                    <div className={styles.strategyHeader}>
                      <span>{item.id}</span>
                      <div className={styles.strategyTags}>
                        {item.tags?.map((tag) => (
                          <Tag key={`${item.id}-${tag}`} color="default">
                            {tag}
                          </Tag>
                        ))}
                      </div>
                    </div>
                  }
                  description={
                    <div>
                      <div className={styles.strategyDescription}>{item.description}</div>
                      <div className={styles.strategyMeta}>
                        <Tag color={actionTagColor(item.action.kind)}>
                          {item.action.kind}
                        </Tag>
                        {item.telemetry_label && (
                          <Tag color="geekblue">{item.telemetry_label}</Tag>
                        )}
                        <span>{describeAction(item)}</span>
                      </div>
                    </div>
                  }
                />
              </List.Item>
            )}
          />
        ) : (
          <Paragraph className={styles.hintText}>
            暂无策略。确认后端已升级到 Stage-2 自愈配置。
          </Paragraph>
        )}
      </Card>

      <Card
        className={styles.sectionCard}
        title={
          <div className={styles.cardTitle}>
            <AppstoreAddOutlined className={styles.cardTitleIcon} />
            插件注册表
          </div>
        }
        extra={
          <Button type="link" onClick={() => loadPluginRegistry()} loading={pluginLoading}>
            刷新
          </Button>
        }
      >
        {pluginLoading ? (
          <Spin />
        ) : filteredPlugins.length ? (
          <Space direction="vertical" style={{ width: '100%' }} size="middle">
            <div className={styles.pluginFilters}>
              <Input
                allowClear
                prefix={<SearchOutlined />}
                placeholder="搜索插件/Helper"
                value={pluginSearch}
                onChange={(e) => setPluginSearch(e.target.value)}
              />
              <Select
                allowClear
                placeholder="状态"
                value={pluginStatusFilter}
                onChange={(value) => setPluginStatusFilter(value || undefined)}
                options={[
                  { label: 'active', value: 'active' },
                  { label: 'pending', value: 'pending' },
                  { label: 'disabled', value: 'disabled' },
                ]}
                style={{ width: 160 }}
              />
              <Button type="link" onClick={() => loadPluginRegistry()}>
                刷新
              </Button>
            </div>
            {pluginStats && (
              <div className={styles.pluginStatsGrid}>
                <Statistic title="总插件" value={pluginStats.total_registry_entries} />
                <Statistic title="已启用" value={pluginStats.active_plugins} />
                <Statistic title="待复核" value={pluginStats.pending_review} />
                <Statistic
                  title="最近审阅"
                  value={pluginStats.last_reviewed_at ?? '—'}
                  valueStyle={{ fontSize: 13 }}
                />
              </div>
            )}
            <List
              itemLayout="vertical"
              dataSource={filteredPlugins}
              renderItem={(plugin) => (
                <List.Item
                  key={plugin.id}
                  className={styles.pluginRow}
                  actions={[
                    <Button
                      key={`${plugin.id}-add-helper`}
                      type="link"
                      onClick={() => openCreateHelperModal(plugin.id)}
                    >
                      新增 Helper
                    </Button>,
                    <Select
                      key={`${plugin.id}-status`}
                      size="small"
                      value={(plugin.status ?? 'pending').toLowerCase()}
                      onChange={(next) =>
                        handlePluginStatusChange(
                          plugin,
                          next as 'active' | 'pending' | 'disabled'
                        )
                      }
                      options={[
                        { label: 'active', value: 'active' },
                        { label: 'pending', value: 'pending' },
                        { label: 'disabled', value: 'disabled' },
                      ]}
                    />,
                  ]}
                >
                  <List.Item.Meta
                    title={
                      <div className={styles.strategyHeader}>
                        <span>{plugin.id}</span>
                        <div className={styles.pluginScopes}>
                          {plugin.scopes?.map((scope) => (
                            <Tag key={`${plugin.id}-${scope}`} color="processing">
                              {scope}
                            </Tag>
                          ))}
                        </div>
                      </div>
                    }
                    description={
                      <div>
                        <div className={styles.strategyDescription}>
                          {plugin.description || '—'}
                        </div>
                        <div className={styles.pluginMeta}>
                          <Tag color="default">{(plugin.status || 'pending').toLowerCase()}</Tag>
                          {plugin.owner && <Tag color="magenta">{plugin.owner}</Tag>}
                          <span>
                            reviewed:{' '}
                            {plugin.last_reviewed_at || '—'}
                          </span>
                        </div>
                        {plugin.helpers?.length ? (
                          <div className={styles.helperList}>
                            <span>Helpers:</span>
                            {plugin.helpers.map((helper) => (
                              <div key={`${plugin.id}-${helper.id}`} className={styles.helperItem}>
                                <Tag color={helper.auto_insert ? 'green' : 'default'}>
                                  {helper.auto_insert ? 'auto' : 'hint'}
                                </Tag>
                                <div className={styles.helperDetails}>
                                  <div>
                                    {helper.id}{' '}
                                    <span className={styles.helperMetaText}>
                                      ({helper.steps?.length ?? 0} step{helper.steps && helper.steps.length === 1 ? '' : 's'})
                                    </span>
                                  </div>
                                  <div className={styles.helperMetaText}>
                                    {helper.description || helper.prompt || '—'}
                                    {helper.blockers?.length
                                      ? ` · blockers: ${helper.blockers.join(', ')}`
                                      : ''}
                                  </div>
                                </div>
                                <Space size="small">
                                  <Button
                                    size="small"
                                    type="link"
                                    onClick={() => openEditHelperModal(plugin.id, helper)}
                                  >
                                    编辑
                                  </Button>
                                  <Button
                                    size="small"
                                    type="link"
                                    danger
                                    onClick={() => handleHelperDelete(plugin.id, helper.id)}
                                  >
                                    删除
                                  </Button>
                                </Space>
                              </div>
                            ))}
                          </div>
                        ) : null}
                      </div>
                    }
                  />
                </List.Item>
              )}
            />
          </Space>
        ) : (
          <Paragraph className={styles.hintText}>
            {pluginRegistry.length
              ? '没有符合筛选条件的插件。'
              : '暂无插件。确保 registry.json 已配置。'}
          </Paragraph>
        )}
      </Card>

      <Modal
        title={editingHelper ? `编辑 Helper ${editingHelper.id}` : '新增 Helper'}
        open={helperModalVisible}
        onCancel={closeHelperModal}
        onOk={handleHelperSubmit}
        confirmLoading={helperSaving}
        width={720}
        destroyOnClose
      >
        <Form
          form={helperForm}
          layout="vertical"
          initialValues={buildDefaultHelperValues()}
          onValuesChange={handleHelperFormChange}
        >
          <Form.Item
            label="ID"
            name="id"
            rules={[{ required: true, message: '请输入 Helper ID' }]}
          >
            <Input disabled={!!editingHelper} placeholder="helper_id" allowClear />
          </Form.Item>
          <Form.Item
            label="URL 正则"
            name="pattern"
            rules={[{ required: true, message: '请输入匹配正则' }]}
          >
            <Input placeholder="https?://(www\\.)?example\\." allowClear />
          </Form.Item>
          <Form.Item label="描述" name="description">
            <Input.TextArea rows={2} placeholder="描述 Helper 的用途" allowClear />
          </Form.Item>
          <Form.Item label="提示文本" name="prompt">
            <Input.TextArea rows={2} placeholder="Planner 提示（可选）" allowClear />
          </Form.Item>
          <Form.Item label="阻塞标签" name="blockers">
            <Select mode="tags" placeholder="blocker 标签" allowClear />
          </Form.Item>
          <Space size="large" align="center">
            <Form.Item label="自动插入" name="auto_insert" valuePropName="checked">
              <Switch />
            </Form.Item>
            <Form.Item label="URL 必须包含" name="url_includes">
              <Select mode="tags" placeholder="包含关键字" style={{ minWidth: 200 }} />
            </Form.Item>
            <Form.Item label="URL 排除" name="url_excludes">
              <Select mode="tags" placeholder="排除关键字" style={{ minWidth: 200 }} />
            </Form.Item>
          </Space>
          <Divider orientation="left">步骤</Divider>
          <Form.List name="steps">
            {(fields, { add, remove }) => (
              <div className={styles.helperSteps}>
                {fields.map((field) => (
                  <Card
                    key={field.key}
                    size="small"
                    className={styles.helperStepCard}
                    title={`步骤 ${field.name + 1}`}
                    extra={
                      fields.length > 1 ? (
                        <Button
                          type="link"
                          danger
                          icon={<MinusCircleOutlined />}
                          onClick={() => remove(field.name)}
                        >
                          删除
                        </Button>
                      ) : null
                    }
                  >
                    <Form.Item
                      {...field}
                      label="标题"
                      name={[field.name, 'title']}
                      fieldKey={[field.fieldKey!, 'title']}
                      rules={[{ required: true, message: '请输入步骤标题' }]}
                    >
                      <Input placeholder="例如：点击接受按钮" />
                    </Form.Item>
                    <Form.Item
                      {...field}
                      label="描述"
                      name={[field.name, 'detail']}
                      fieldKey={[field.fieldKey!, 'detail']}
                    >
                      <Input placeholder="可选" />
                    </Form.Item>
                    <Space size="large" wrap>
                      <Form.Item
                        {...field}
                        label="等待策略"
                        name={[field.name, 'wait']}
                        fieldKey={[field.fieldKey!, 'wait']}
                        initialValue="dom_ready"
                      >
                        <Select
                          style={{ width: 160 }}
                          options={[
                            { label: 'DOM Ready', value: 'dom_ready' },
                            { label: 'Idle', value: 'idle' },
                          ]}
                        />
                      </Form.Item>
                      <Form.Item
                        {...field}
                        label="超时 (ms)"
                        name={[field.name, 'timeout_ms']}
                        fieldKey={[field.fieldKey!, 'timeout_ms']}
                      >
                        <InputNumber min={0} placeholder="可选" />
                      </Form.Item>
                      <Form.Item
                        {...field}
                        label="工具类型"
                        name={[field.name, 'tool_type']}
                        fieldKey={[field.fieldKey!, 'tool_type']}
                        initialValue="click_css"
                        rules={[{ required: true, message: '请选择工具类型' }]}
                      >
                        <Select
                          style={{ width: 180 }}
                          options={[
                            { label: 'Click (CSS)', value: 'click_css' },
                            { label: 'Click (Text)', value: 'click_text' },
                            { label: 'Custom Tool', value: 'custom' },
                          ]}
                        />
                      </Form.Item>
                    </Space>
                    <Form.Item
                      noStyle
                      shouldUpdate={(prev, cur) =>
                        prev.steps?.[field.name]?.tool_type !==
                        cur.steps?.[field.name]?.tool_type
                      }
                    >
                      {() => {
                        const toolType = helperForm.getFieldValue([
                          'steps',
                          field.name,
                          'tool_type',
                        ]) as HelperStepFormValues['tool_type'];
                        switch (toolType) {
                          case 'click_text':
                            return (
                              <Space size="large" wrap>
                                <Form.Item
                                  {...field}
                                  label="文本"
                                  name={[field.name, 'text']}
                                  fieldKey={[field.fieldKey!, 'text']}
                                  rules={[{ required: true, message: '请输入文本内容' }]}
                                >
                                  <Input placeholder="例如：Accept all" />
                                </Form.Item>
                                <Form.Item
                                  {...field}
                                  label="完全匹配"
                                  name={[field.name, 'exact']}
                                  fieldKey={[field.fieldKey!, 'exact']}
                                  valuePropName="checked"
                                >
                                  <Switch size="small" />
                                </Form.Item>
                              </Space>
                            );
                          case 'custom':
                            return (
                              <Space direction="vertical" style={{ width: '100%' }}>
                                <Form.Item
                                  {...field}
                                  label="自定义工具名称"
                                  name={[field.name, 'name']}
                                  fieldKey={[field.fieldKey!, 'name']}
                                  rules={[{ required: true, message: '请输入工具名称' }]}
                                >
                                  <Input placeholder="agent.custom-action" />
                                </Form.Item>
                                <Form.Item
                                  {...field}
                                  label="Payload (JSON)"
                                  name={[field.name, 'payload']}
                                  fieldKey={[field.fieldKey!, 'payload']}
                                >
                                  <Input.TextArea rows={3} placeholder={'{ "key": "value" }'} />
                                </Form.Item>
                              </Space>
                            );
                          default:
                            return (
                              <Form.Item
                                {...field}
                                label="CSS 选择器"
                                name={[field.name, 'selector']}
                                fieldKey={[field.fieldKey!, 'selector']}
                                rules={[{ required: true, message: '请输入 CSS 选择器' }]}
                              >
                                <Input placeholder="#accept" />
                              </Form.Item>
                            );
                        }
                      }}
                    </Form.Item>
                  </Card>
                ))}
                <Button
                  type="dashed"
                  block
                  icon={<PlusOutlined />}
                  onClick={() => add(defaultHelperStep())}
                >
                  添加步骤
                </Button>
              </div>
            )}
          </Form.List>
        </Form>
        <Divider orientation="left">JSON 预览</Divider>
        {helperPreviewError ? (
          <Alert type="error" message={helperPreviewError} showIcon />
        ) : helperPreview ? (
          <pre className={styles.resultBlock}>{helperPreview}</pre>
        ) : (
          <Paragraph className={styles.hintText}>
            编辑表单以查看 Helper JSON 预览。
          </Paragraph>
        )}
      </Modal>

      <Card
        className={styles.sectionCard}
        title={
          <div className={styles.cardTitle}>
            <DatabaseOutlined className={styles.cardTitleIcon} />
            录制会话
          </div>
        }
        extra={
          <Space>
            <Select
              allowClear
              placeholder="状态"
              value={recordingsStateFilter}
              style={{ width: 160 }}
              onChange={(value) => setRecordingsStateFilter(value || undefined)}
              options={[
                { label: 'recording', value: 'recording' },
                { label: 'completed', value: 'completed' },
              ]}
            />
            <InputNumber
              min={1}
              max={50}
              value={recordingsLimit}
              onChange={(value) => setRecordingsLimit(value ?? 10)}
            />
            <Button type="link" onClick={() => loadRecordings()} loading={recordingsLoading}>
              刷新
            </Button>
          </Space>
        }
      >
        {recordingsLoading ? (
          <Spin />
        ) : recordings.length ? (
          <List
            size="small"
            dataSource={recordings}
            renderItem={(item) => (
              <List.Item
                actions={[
                  <Button type="link" key="view" onClick={() => loadRecordingDetail(item.id)}>
                    查看详情
                  </Button>,
                ]}
              >
                <List.Item.Meta
                  title={`${item.id} (${item.state})`}
                  description={`updated: ${item.updated_at} · plan: ${item.has_agent_plan ? 'YES' : '—'}`}
                />
              </List.Item>
            )}
          />
        ) : (
          <Paragraph className={styles.hintText}>暂无录制会话</Paragraph>
        )}
        {recordingDetailLoading ? (
          <Spin style={{ marginTop: 12 }} />
        ) : recordingDetail ? (
          <>
            <Space style={{ marginTop: 12, marginBottom: 12 }}>
              <Button
                type="primary"
                icon={<DownloadOutlined />}
                disabled={!recordingDetail.metadata?.agent_plan}
                onClick={() => handleDownloadRecordingPlan(recordingDetail)}
              >
                下载计划 JSON
              </Button>
            </Space>
            <pre className={styles.resultBlock}>{JSON.stringify(recordingDetail, null, 2)}</pre>
          </>
        ) : null}
      </Card>

      <Card
        className={styles.sectionCard}
        title={
          <div className={styles.cardTitle}>
            <EyeOutlined className={styles.cardTitleIcon} />
            感知服务指标
          </div>
        }
        extra={
          <Button type="link" onClick={() => loadPerceptionMetrics()} loading={metricsLoading}>
            刷新
          </Button>
        }
      >
        {metrics ? (
          <Descriptions column={2} bordered size="small">
            <Descriptions.Item label="总调用次数">{metrics.total_runs}</Descriptions.Item>
            <Descriptions.Item label="平均时长 (ms)">
              {metrics.avg_duration_ms.toFixed(2)}
            </Descriptions.Item>
            <Descriptions.Item label="共享命中">{metrics.shared_hits}</Descriptions.Item>
            <Descriptions.Item label="共享未命中">{metrics.shared_misses}</Descriptions.Item>
            <Descriptions.Item label="共享失败">{metrics.shared_failures}</Descriptions.Item>
            <Descriptions.Item label="临时会话">{metrics.ephemeral_runs}</Descriptions.Item>
            <Descriptions.Item label="失败次数">{metrics.failed_runs}</Descriptions.Item>
          </Descriptions>
        ) : (
          <Paragraph className={styles.hintText}>
            暂无指标数据。执行一次感知请求或点击刷新按钮以查看最新统计。
          </Paragraph>
        )}
      </Card>

      <Card
        className={styles.sectionCard}
        title={
          <div className={styles.cardTitle}>
            <HeartOutlined className={styles.cardTitleIcon} />
            健康检查 /health
          </div>
        }
        extra={
          <Button type="primary" loading={healthState === 'loading'} onClick={runHealthCheck}>
            立即检测
          </Button>
        }
      >
        {healthState !== 'idle' ? (
          <Alert
            type={healthState === 'success' ? 'success' : 'error'}
            message={healthState === 'success' ? '后端在线' : '无法连接后端'}
            description={healthResult}
            showIcon
          />
        ) : (
          <Paragraph className={styles.hintText}>点击“立即检测”确认后端是否可达。</Paragraph>
        )}
      </Card>

      <Card
        className={styles.sectionCard}
        title={
          <div className={styles.cardTitle}>
            <RobotOutlined className={styles.cardTitleIcon} />
            Chat API 快速测试
          </div>
        }
        extra={
          <Button type="primary" loading={chatLoading} onClick={runChatTest}>
            发送测试请求
          </Button>
        }
      >
        <Space direction="vertical" style={{ width: '100%' }}>
          <TextArea
            rows={3}
            value={chatPrompt}
            onChange={(e) => setChatPrompt(e.target.value)}
            placeholder="Describe the goal..."
          />
          {chatError && <Alert type="error" message={chatError} showIcon />}
          {chatOutput && renderResultBlock(chatOutput)}
          {!chatOutput && !chatError && (
            <Paragraph className={styles.hintText}>
              将调用 POST /api/chat，并展示原始 JSON 响应。
            </Paragraph>
          )}
        </Space>
      </Card>

      <Card
        className={styles.sectionCard}
        title={
          <div className={styles.cardTitle}>
            <EyeOutlined className={styles.cardTitleIcon} />
            Perceive API 快速测试
          </div>
        }
        extra={
          <Button type="primary" loading={perceiveLoading} onClick={runPerceiveTest}>
            执行感知
          </Button>
        }
      >
        <Space direction="vertical" style={{ width: '100%' }} size="middle">
          <Input
            value={perceiveUrl}
            onChange={(e) => setPerceiveUrl(e.target.value)}
            placeholder="https://example.com"
            addonBefore="URL"
          />
          <Space className={styles.inlineForm}>
            <Select
              value={perceiveMode}
              onChange={(value) => setPerceiveMode(value)}
              options={[
                { label: 'Structural', value: 'structural' },
                { label: 'Visual', value: 'visual' },
                { label: 'Semantic', value: 'semantic' },
                { label: 'All', value: 'all' },
              ]}
            />
            <InputNumber
              min={30}
              max={240}
              value={perceiveTimeout}
              onChange={(val) => setPerceiveTimeout(typeof val === 'number' ? val : 60)}
              addonAfter="s"
              placeholder="超时时间"
            />
          </Space>
          {perceiveError && <Alert type="error" message={perceiveError} showIcon />}
          {perceiveOutput && renderResultBlock(perceiveOutput)}
          {!perceiveOutput && !perceiveError && (
            <Paragraph className={styles.hintText}>
              感知请求可能需要 1-2 分钟。执行结果会包含 stdout/stderr，便于排查。
            </Paragraph>
          )}
        </Space>
      </Card>
    </div>
  );
}
