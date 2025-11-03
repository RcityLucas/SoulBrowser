import { Card, List, Tag } from 'antd';
import {
  FormOutlined,
  SearchOutlined,
  ExperimentOutlined,
  LineChartOutlined,
  PlusOutlined,
} from '@ant-design/icons';
import styles from './TemplateSelector.module.css';

interface Template {
  id: string;
  title: string;
  description: string;
  icon: React.ReactNode;
  prompt: string;
  tags: string[];
}

const templates: Template[] = [
  {
    id: 'form-fill',
    title: '表单自动填写',
    description: '自动填写网页表单',
    icon: <FormOutlined />,
    prompt: '帮我填写 example.com 的联系表单，姓名填 "张三"，邮箱填 "zhangsan@example.com"',
    tags: ['表单', '常用'],
  },
  {
    id: 'data-scraping',
    title: '网页数据采集',
    description: '从网页提取数据',
    icon: <SearchOutlined />,
    prompt: '从 example.com 采集产品列表，包括产品名称、价格和图片链接',
    tags: ['采集', '数据'],
  },
  {
    id: 'auto-test',
    title: '自动化测试',
    description: '执行自动化测试流程',
    icon: <ExperimentOutlined />,
    prompt: '测试 example.com 的登录功能，使用用户名 "test" 和密码 "123456"',
    tags: ['测试', '质量'],
  },
  {
    id: 'monitoring',
    title: '竞品监控',
    description: '监控竞品价格变化',
    icon: <LineChartOutlined />,
    prompt: '监控 competitor.com 的产品价格，每天检查一次并报告变化',
    tags: ['监控', '定时'],
  },
];

interface Props {
  onSelect: (prompt: string) => void;
}

export default function TemplateSelector({ onSelect }: Props) {
  return (
    <Card title="任务模板" className={styles.card}>
      <List
        dataSource={templates}
        renderItem={(template) => (
          <List.Item
            className={styles.templateItem}
            onClick={() => onSelect(template.prompt)}
          >
            <div className={styles.templateIcon}>{template.icon}</div>
            <div className={styles.templateContent}>
              <div className={styles.templateTitle}>{template.title}</div>
              <div className={styles.templateDescription}>{template.description}</div>
              <div className={styles.templateTags}>
                {template.tags.map((tag) => (
                  <Tag key={tag} size="small">
                    {tag}
                  </Tag>
                ))}
              </div>
            </div>
          </List.Item>
        )}
      />

      <div className={styles.createCustom} onClick={() => onSelect('')}>
        <PlusOutlined />
        <span>自定义任务</span>
      </div>
    </Card>
  );
}
