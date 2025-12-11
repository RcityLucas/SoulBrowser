import { Card, Typography, List, Tag, Descriptions } from 'antd';
import type { ReactNode } from 'react';
import styles from './ExecutionResultCard.module.css';
import type { ExecutionResultEntry } from '@/utils/executionSummary';

interface Props {
  results: ExecutionResultEntry[];
}

export default function ExecutionResultCard({ results }: Props) {
  if (!results.length) {
    return null;
  }

  return (
    <Card size="small" className={styles.resultCard} bordered>
      <Typography.Title level={5}>执行结果</Typography.Title>
      <List
        size="small"
        dataSource={results}
        renderItem={(item) => (
          <List.Item className={styles.resultItem}>
            <div className={styles.resultHeader}>
              <Typography.Text strong>{item.label}</Typography.Text>
              {item.artifactPath && (
                <Tag color="blue">
                  <Typography.Link href={item.artifactPath} target="_blank" rel="noreferrer">
                    下载产物
                  </Typography.Link>
                </Tag>
              )}
            </div>
            {item.data && <div className={styles.resultBody}>{renderValue(item.data)}</div>}
          </List.Item>
        )}
      />
    </Card>
  );
}

export type { ExecutionResultEntry };

type SimpleValue = string | number | boolean | null;

const formatLabel = (label: string) =>
  label
    .replace(/_/g, ' ')
    .replace(/\b\w/g, (char) => char.toUpperCase());

const isPlainObject = (value: unknown): value is Record<string, unknown> =>
  Boolean(value) && typeof value === 'object' && !Array.isArray(value);

const isSimpleRecord = (value: Record<string, unknown>): value is Record<string, SimpleValue> =>
  Object.values(value).every((entry) =>
    entry === null || ['string', 'number', 'boolean'].includes(typeof entry)
  );

const renderParagraph = (text: string) => (
  <Typography.Paragraph
    className={styles.textBlock}
    ellipsis={{ rows: 3, expandable: true, symbol: '展开' }}
    copyable
  >
    {text}
  </Typography.Paragraph>
);

const renderRecord = (record: Record<string, unknown>) => (
  <Descriptions
    column={1}
    size="small"
    bordered
    className={styles.descriptions}
  >
    {Object.entries(record).map(([key, value]) => (
      <Descriptions.Item label={formatLabel(key)} key={key}>
        {renderValue(value)}
      </Descriptions.Item>
    ))}
  </Descriptions>
);

const renderArray = (value: unknown[]) => {
  if (value.length === 0) {
    return <Typography.Text type="secondary">无数据</Typography.Text>;
  }

  const allStrings = value.every((entry) => typeof entry === 'string');
  if (allStrings) {
    return (
      <ul className={styles.simpleList}>
        {value.map((entry, idx) => (
          <li key={`${entry}-${idx}`}>{entry as string}</li>
        ))}
      </ul>
    );
  }

  return (
    <pre className={styles.resultContent}>{JSON.stringify(value, null, 2)}</pre>
  );
};

const renderValue = (value: unknown): ReactNode => {
  if (value === null || value === undefined) {
    return <Typography.Text type="secondary">—</Typography.Text>;
  }

  if (typeof value === 'string') {
    return renderParagraph(value);
  }

  if (typeof value === 'number' || typeof value === 'boolean') {
    return <Typography.Text>{String(value)}</Typography.Text>;
  }

  if (Array.isArray(value)) {
    return renderArray(value);
  }

  if (isPlainObject(value)) {
    return isSimpleRecord(value)
      ? renderRecord(value)
      : (
        <pre className={styles.resultContent}>{JSON.stringify(value, null, 2)}</pre>
      );
  }

  return <pre className={styles.resultContent}>{JSON.stringify(value, null, 2)}</pre>;
};
