import { useState, useRef, useEffect, useCallback, useMemo } from 'react';
import { Input, Button, Card, Space, Tag, Spin, Select, Tooltip, message } from 'antd';
import {
  SendOutlined,
  RobotOutlined,
  UserOutlined,
  PlusOutlined,
  ReloadOutlined,
  LinkOutlined,
} from '@ant-design/icons';
import { useChatStore } from '@/stores/chatStore';
import TaskPlanCard from './TaskPlanCard';
import ExecutionSummaryCard from './ExecutionSummaryCard';
import ExecutionResultCard from './ExecutionResultCard';
import LiveSessionPreview from './LiveSessionPreview';
import BackendStatusBar from '@/components/common/BackendStatusBar';
import { buildExecutionSummary, extractExecutionResults } from '@/utils/executionSummary';
import TemplateSelector from './TemplateSelector';
import styles from './ChatPage.module.css';
import { soulbrowserAPI } from '@/api/soulbrowser';
import type { ChatResponse } from '@/api/soulbrowser';
import type { SessionRecord } from '@/types';
import { useSearchParams } from 'react-router-dom';

const { TextArea } = Input;

const quickPrompts = [
  {
    label: '采集新品',
    prompt: '从 example.com 抓取最新 10 个产品的名称、价格和图片链接',
  },
  {
    label: '检测登录',
    prompt: '测试 example.com 的登录流程，尝试使用 demo/demo123 并报告错误提示',
  },
  {
    label: '监控价格',
    prompt: '监控 competitor.com 的主力商品价位，如果有变化要告警',
  },
];

const remedyCommands = [
  {
    label: '抓取当前页面',
    prompt: '抓取当前页面并输出结构化摘要',
    preset: 'capture',
  },
  {
    label: '总结上一页',
    prompt: '总结上一页的关键信息并生成要点',
    preset: 'summarize',
  },
];

export default function ChatPage() {
  const [input, setInput] = useState('');
  const messagesEndRef = useRef<HTMLDivElement>(null);
  const { messages, isTyping, addMessage, setTyping, setCurrentPlan } = useChatStore();
  const [searchParams, setSearchParams] = useSearchParams();
  const [sessions, setSessions] = useState<SessionRecord[]>([]);
  const [sessionsLoading, setSessionsLoading] = useState(false);
  const [creatingSession, setCreatingSession] = useState(false);
  const [selectedSessionId, setSelectedSessionId] = useState<string | null>(
    searchParams.get('session')
  );
  const [sessionCapabilities, setSessionCapabilities] = useState<Record<string, { hasActivePage: boolean }>>({});

  const applySessionSelection = useCallback(
    (sessionId: string | null) => {
      setSelectedSessionId(sessionId);
      const next = new URLSearchParams(searchParams);
      if (sessionId) {
        next.set('session', sessionId);
      } else {
        next.delete('session');
      }
      setSearchParams(next);
    },
    [searchParams, setSearchParams]
  );

  const loadSessions = useCallback(async () => {
    setSessionsLoading(true);
    try {
      const list = await soulbrowserAPI.listSessions();
      setSessions(list);
    } catch (err) {
      console.error(err);
      message.error('加载会话列表失败');
    } finally {
      setSessionsLoading(false);
    }
  }, []);

  useEffect(() => {
    void loadSessions();
  }, [loadSessions]);

  useEffect(() => {
    const querySession = searchParams.get('session');
    if (querySession !== selectedSessionId) {
      setSelectedSessionId(querySession);
    }
  }, [searchParams, selectedSessionId]);

  const handleSessionChange = (value: string) => {
    if (value === 'auto') {
      applySessionSelection(null);
      return;
    }
    applySessionSelection(value);
  };

  const handleCreateSession = useCallback(async () => {
    setCreatingSession(true);
    try {
      const record = await soulbrowserAPI.createSession();
      message.success('已创建新会话');
      setSessions((prev) => [record, ...prev.filter((item) => item.id !== record.id)]);
      applySessionSelection(record.id);
    } catch (err) {
      console.error(err);
      message.error('创建会话失败');
    } finally {
      setCreatingSession(false);
    }
  }, [applySessionSelection]);

  const sessionOptions = useMemo(() => {
    const options = sessions.map((session) => ({
      value: session.id,
      label: `${session.profile_label || session.id.slice(0, 8)} · ${session.status}`,
    }));
    if (selectedSessionId && !options.some((opt) => opt.value === selectedSessionId)) {
      options.unshift({
        value: selectedSessionId,
        label: `${selectedSessionId.slice(0, 8)} · 待刷新`,
      });
    }
    return options;
  }, [sessions, selectedSessionId]);

  const sessionLink = selectedSessionId
    ? `/sessions?focus=${selectedSessionId}`
    : '/sessions';
  const activeSession = selectedSessionId
    ? sessions.find((session) => session.id === selectedSessionId)
    : undefined;

  const scrollToBottom = () => {
    messagesEndRef.current?.scrollIntoView({ behavior: 'smooth' });
  };

  useEffect(() => {
    scrollToBottom();
  }, [messages]);

  const deliverAssistantMessage = useCallback(
    (response: ChatResponse) => {
      const content =
        response.stdout?.trim() ||
        (response.plan ? '已生成任务计划，请查看详情。' : '任务已提交，稍后查看结果。');

    const suggestions = Array.isArray((response.flow as any)?.suggestions)
      ? (response.flow as any).suggestions
      : undefined;

    const executionSummary = buildExecutionSummary(
      response.flow,
      response.success,
      response.stdout ?? undefined,
      response.stderr ?? undefined
    );
    const executionResults = extractExecutionResults((response.flow as any)?.execution);

    addMessage({
      role: 'assistant',
      content,
      taskPlan: response.plan ?? undefined,
      suggestions,
      executionSummary,
      executionResults,
    });

      if (response.plan) {
        setCurrentPlan(response.plan);
      }
    },
    [addMessage, setCurrentPlan]
  );

  const executePrompt = useCallback(
    async (rawPrompt: string, options?: { keepInput?: boolean }) => {
      const prompt = rawPrompt.trim();
      if (!prompt) return;

      addMessage({
        role: 'user',
        content: prompt,
      });

      setTyping(true);
      if (!options?.keepInput) {
        setInput('');
      }

      let sessionIdForRequest: string | undefined = undefined;
      if (selectedSessionId) {
        const sessionReady = sessionCapabilities[selectedSessionId]?.hasActivePage;
        if (sessionReady) {
          sessionIdForRequest = selectedSessionId;
        } else {
          message.warning('所选会话没有活跃页面，已回退到自动模式。');
        }
      }

      if (!sessionIdForRequest) {
        try {
          const session = await soulbrowserAPI.createSession();
          sessionIdForRequest = session.id;
          setSessions((prev) => [session, ...prev.filter((item) => item.id !== session.id)]);
          applySessionSelection(session.id);
        } catch (err) {
          console.error(err);
          message.error('创建实时会话失败');
        }
      }

      try {
        const defaultPlanner = import.meta.env.VITE_DEFAULT_PLANNER ?? 'llm';
        const plannerPayload: Record<string, unknown> = {
          planner: defaultPlanner,
        };

        if (defaultPlanner === 'llm') {
          plannerPayload.llm_provider =
            import.meta.env.VITE_DEFAULT_LLM_PROVIDER ?? 'openai';

          if (import.meta.env.VITE_DEFAULT_LLM_MODEL) {
            plannerPayload.llm_model = import.meta.env.VITE_DEFAULT_LLM_MODEL;
          }
          if (import.meta.env.VITE_LLM_API_BASE) {
            plannerPayload.llm_api_base = import.meta.env.VITE_LLM_API_BASE;
          }
          if (import.meta.env.VITE_LLM_TEMPERATURE) {
            const value = Number(import.meta.env.VITE_LLM_TEMPERATURE);
            if (!Number.isNaN(value)) {
              plannerPayload.llm_temperature = value;
            }
          }
          if (import.meta.env.VITE_LLM_MAX_OUTPUT_TOKENS) {
            const value = Number(import.meta.env.VITE_LLM_MAX_OUTPUT_TOKENS);
            if (!Number.isNaN(value)) {
              plannerPayload.llm_max_output_tokens = value;
            }
          }
        }

        const response = await soulbrowserAPI.chat({
          prompt,
          execute: true,
          session_id: sessionIdForRequest,
          ...plannerPayload,
        });

        if (!response.success) {
          throw new Error(response.stderr || '执行失败');
        }

        deliverAssistantMessage(response);
        if (response.session_id) {
          applySessionSelection(response.session_id);
          void loadSessions();
        }
      } catch (err) {
        const messageText = err instanceof Error ? err.message : '请求失败';
        addMessage({
          role: 'assistant',
          content: `⚠️ ${messageText}`,
        });
        console.error(err);
      } finally {
        setTyping(false);
      }
    },
    [
      addMessage,
      setTyping,
      setInput,
      deliverAssistantMessage,
      selectedSessionId,
      sessionCapabilities,
      applySessionSelection,
      loadSessions,
    ]
  );

  const handleSend = async (overridePrompt?: string) => {
    const promptValue = overridePrompt ?? input;
    if (!promptValue.trim()) {
      return;
    }
    await executePrompt(promptValue, { keepInput: Boolean(overridePrompt) });
  };

  const handleKeyPress = (e: React.KeyboardEvent) => {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      void handleSend();
    }
  };

  const handleTemplateSelect = (template: string) => {
    setInput(template);
  };

  useEffect(() => {
    const preset = searchParams.get('preset');
    if (!preset) {
      return;
    }
    const command = remedyCommands.find((cmd) => cmd.preset === preset);
    if (command) {
      void executePrompt(command.prompt, { keepInput: true });
    }
    const next = new URLSearchParams(searchParams);
    next.delete('preset');
    next.delete('fromTask');
    setSearchParams(next);
  }, [executePrompt, searchParams, setSearchParams]);

  const defaultPlanner = (import.meta.env.VITE_DEFAULT_PLANNER ?? 'llm').toUpperCase();
  const defaultProvider = (import.meta.env.VITE_DEFAULT_LLM_PROVIDER ?? 'openai').toUpperCase();

  const handleSessionPreviewUpdate = useCallback((sessionId: string, hasFrame: boolean) => {
    setSessionCapabilities((prev) => {
      const previous = prev[sessionId]?.hasActivePage;
      if (previous === hasFrame) {
        return prev;
      }
      return {
        ...prev,
        [sessionId]: { hasActivePage: hasFrame },
      };
    });
  }, []);

  return (
    <div className={styles.chatPage}>
      <div className={styles.sidebar}>
        <TemplateSelector onSelect={handleTemplateSelect} />
      </div>

      <div className={styles.chatPanel}>
        <Card className={styles.sessionCard} size="small" title="持久会话">
          <div className={styles.sessionControls}>
            <Select
              value={selectedSessionId ?? 'auto'}
              onChange={handleSessionChange}
              loading={sessionsLoading}
              showSearch
              optionFilterProp="label"
              options={[
                { value: 'auto', label: '自动创建会话（每次执行都会生成新的浏览器状态）' },
                ...sessionOptions,
              ]}
              style={{ minWidth: 280 }}
            />
            <div className={styles.sessionActions}>
              <Button
                size="small"
                icon={<PlusOutlined />}
                loading={creatingSession}
                onClick={() => void handleCreateSession()}
              >
                创建会话
              </Button>
              <Button
                size="small"
                icon={<ReloadOutlined />}
                loading={sessionsLoading}
                onClick={() => void loadSessions()}
              >
                刷新
              </Button>
              <Tooltip title="打开会话面板查看更多执行细节">
                <Button size="small" icon={<LinkOutlined />} href={sessionLink} target="_blank">
                  会话列表
                </Button>
              </Tooltip>
            </div>
            <span>
              {selectedSessionId
                ? `当前会话：${
                    activeSession?.profile_label || `${selectedSessionId.slice(0, 8)}...`
                  }`
                : '当前处于自动模式，执行任务时系统会自动创建会话。'}
            </span>
          </div>
        </Card>
        <div className={styles.chatHero}>
          <div className={styles.heroCopy}>
            <p className={styles.heroEyebrow}>Aurora Agent Console</p>
            <h2>通过对话 orchestrate 浏览器工作流</h2>
            <p className={styles.heroDescription}>
              描述你的研究或操作目标，系统会输出规划、执行轨迹与结构化交付。试试点击右侧模板或输入自定义需求。
            </p>
          </div>
          <Space wrap className={styles.heroMeta}>
            <Tag bordered={false}>Planner {defaultPlanner}</Tag>
            <Tag bordered={false}>LLM {defaultProvider}</Tag>
            <Tag bordered={false} color="green">
              执行模式 · 自动
            </Tag>
          </Space>
        </div>

        <BackendStatusBar className={styles.statusBar} />

        <Card bordered={false} className={styles.messageBoard}>
          <div className={styles.messages}>
            {messages.length === 0 && (
              <div className={styles.welcome}>
                <RobotOutlined style={{ fontSize: 48, color: '#1890ff' }} />
                <h2>你好！我可以帮你完成网页自动化任务</h2>
                <p>请描述你想要做什么，或者选择一个任务模板</p>
              </div>
            )}

            {messages.map((message) => (
            <div
              key={message.id}
              className={`${styles.message} ${
                message.role === 'user' ? styles.userMessage : styles.assistantMessage
              }`}
            >
              <div className={styles.messageAvatar}>
                {message.role === 'user' ? <UserOutlined /> : <RobotOutlined />}
              </div>
              <div className={styles.messageContent}>
                <div className={styles.messageText}>{message.content}</div>

                {message.taskPlan && (
                  <TaskPlanCard plan={message.taskPlan} className={styles.taskPlanCard} />
                )}

                {message.executionSummary && (
                  <ExecutionSummaryCard summary={message.executionSummary} />
                )}

                {message.executionResults && message.executionResults.length > 0 && (
                  <ExecutionResultCard results={message.executionResults} />
                )}

                {message.suggestions && message.suggestions.length > 0 && (
                  <Space wrap className={styles.suggestions}>
                    {message.suggestions.map((suggestion, index) => (
                      <Tag
                        key={index}
                        onClick={() => setInput(suggestion)}
                        style={{ cursor: 'pointer' }}
                      >
                        {suggestion}
                      </Tag>
                    ))}
                  </Space>
                )}
              </div>
            </div>
          ))}

          {isTyping && (
            <div className={`${styles.message} ${styles.assistantMessage}`}>
              <div className={styles.messageAvatar}>
                <RobotOutlined />
              </div>
              <div className={styles.messageContent}>
                <Spin size="small" /> <span className={styles.typingText}>正在思考...</span>
              </div>
            </div>
          )}

            <div ref={messagesEndRef} />
          </div>
        </Card>

        <div className={styles.quickActions}>
          <span className={styles.quickActionsLabel}>快速指令</span>
          <Space wrap>
            {quickPrompts.map((action) => (
              <Button
                key={action.label}
                type="default"
                ghost
                size="small"
                className={styles.quickActionButton}
                onClick={() => handleTemplateSelect(action.prompt)}
              >
                {action.label}
              </Button>
            ))}
          </Space>
        </div>

        <div className={styles.quickActions}>
          <span className={styles.quickActionsLabel}>补救工具</span>
          <Space wrap>
            {remedyCommands.map((action) => (
              <Button
                key={action.label}
                type="primary"
                ghost
                size="small"
                className={styles.quickActionButton}
                disabled={isTyping}
                onClick={() => void handleSend(action.prompt)}
              >
                {action.label}
              </Button>
            ))}
          </Space>
        </div>

        <div className={styles.inputArea}>
          <Card bordered={false} className={styles.inputCard}>
            <TextArea
              value={input}
              onChange={(e) => setInput(e.target.value)}
              onKeyPress={handleKeyPress}
              placeholder="请输入任务描述... (Shift+Enter 换行)"
              autoSize={{ minRows: 2, maxRows: 6 }}
              className={styles.textarea}
            />
            <div className={styles.inputActions}>
              <Space size={12} wrap>
                <Tag color="cyan" className={styles.inputHint}>
                  Shift + Enter 换行
                </Tag>
                <Button
                  type="primary"
                  icon={<SendOutlined />}
                  onClick={() => void handleSend()}
                  disabled={!input.trim() || isTyping}
                  size="large"
                >
                  发送
                </Button>
              </Space>
            </div>
          </Card>
        </div>
      </div>

      <div className={styles.visualPanel}>
        <LiveSessionPreview
          sessionId={selectedSessionId}
          session={activeSession}
          onSessionSnapshot={handleSessionPreviewUpdate}
        />
      </div>
    </div>
  );
}
