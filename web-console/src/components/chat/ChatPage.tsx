import { useState, useRef, useEffect } from 'react';
import { Input, Button, Card, Space, Tag, Spin } from 'antd';
import { SendOutlined, RobotOutlined, UserOutlined } from '@ant-design/icons';
import { useChatStore } from '@/stores/chatStore';
import { useWebSocket } from '@/hooks/useWebSocket';
import TaskPlanCard from './TaskPlanCard';
import TemplateSelector from './TemplateSelector';
import styles from './ChatPage.module.css';

const { TextArea } = Input;

export default function ChatPage() {
  const [input, setInput] = useState('');
  const messagesEndRef = useRef<HTMLDivElement>(null);
  const { messages, isTyping, currentPlan, addMessage, setTyping, setCurrentPlan } =
    useChatStore();
  const { send, on } = useWebSocket();

  const scrollToBottom = () => {
    messagesEndRef.current?.scrollIntoView({ behavior: 'smooth' });
  };

  useEffect(() => {
    scrollToBottom();
  }, [messages]);

  useEffect(() => {
    const unsubscribe = on('chat_response', (response: any) => {
      setTyping(false);
      addMessage({
        role: 'assistant',
        content: response.content,
        taskPlan: response.taskPlan,
        suggestions: response.suggestions,
      });

      if (response.taskPlan) {
        setCurrentPlan(response.taskPlan);
      }
    });

    return unsubscribe;
  }, [on, addMessage, setTyping, setCurrentPlan]);

  const handleSend = () => {
    if (!input.trim()) return;

    addMessage({
      role: 'user',
      content: input,
    });

    send({
      type: 'chat_message',
      payload: { content: input },
    });

    setTyping(true);
    setInput('');
  };

  const handleKeyPress = (e: React.KeyboardEvent) => {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      handleSend();
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
                onClick={handleSend}
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
