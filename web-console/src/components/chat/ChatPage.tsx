import { useState, useRef, useEffect } from 'react';
import { Input, Button, Card, Space, Tag, Spin } from 'antd';
import { SendOutlined, RobotOutlined, UserOutlined } from '@ant-design/icons';
import { useChatStore } from '@/stores/chatStore';
import TaskPlanCard from './TaskPlanCard';
import ExecutionSummaryCard from './ExecutionSummaryCard';
import ExecutionResultCard from './ExecutionResultCard';
import BackendStatusBar from '@/components/common/BackendStatusBar';
import { buildExecutionSummary, extractExecutionResults } from '@/utils/executionSummary';
import TemplateSelector from './TemplateSelector';
import styles from './ChatPage.module.css';
import { soulbrowserAPI } from '@/api/soulbrowser';
import type { ChatResponse } from '@/api/soulbrowser';

const { TextArea } = Input;

export default function ChatPage() {
  const [input, setInput] = useState('');
  const messagesEndRef = useRef<HTMLDivElement>(null);
  const { messages, isTyping, currentPlan, addMessage, setTyping, setCurrentPlan } =
    useChatStore();

  const scrollToBottom = () => {
    messagesEndRef.current?.scrollIntoView({ behavior: 'smooth' });
  };

  useEffect(() => {
    scrollToBottom();
  }, [messages]);

  const deliverAssistantMessage = (response: ChatResponse) => {
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
  };

  const handleSend = async () => {
    const prompt = input.trim();
    if (!prompt) return;

    addMessage({
      role: 'user',
      content: prompt,
    });

    setTyping(true);
    setInput('');

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
        ...plannerPayload,
      });

      if (!response.success) {
        throw new Error(response.stderr || '执行失败');
      }

      deliverAssistantMessage(response);
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

  return (
    <div className={styles.chatPage}>
      <div className={styles.sidebar}>
        <TemplateSelector onSelect={handleTemplateSelect} />
      </div>

      <div className={styles.chatContainer}>
        <BackendStatusBar className={styles.statusBar} />
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
              <Button
                type="primary"
                icon={<SendOutlined />}
                onClick={() => void handleSend()}
                disabled={!input.trim() || isTyping}
                size="large"
              >
                发送
              </Button>
            </div>
          </Card>
        </div>
      </div>
    </div>
  );
}
