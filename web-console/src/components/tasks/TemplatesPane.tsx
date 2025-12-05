import { useCallback, useEffect, useState } from 'react';
import dayjs from 'dayjs';
import {
  Button,
  Descriptions,
  Empty,
  Form,
  Input,
  InputNumber,
  List,
  Modal,
  Popconfirm,
  Space,
  Spin,
  Tag,
  Typography,
  message,
} from 'antd';
import { PlayCircleOutlined, ReloadOutlined, SaveOutlined, DeleteOutlined, EditOutlined } from '@ant-design/icons';
import soulbrowserAPI, {
  CreateMemoryRecordRequest,
  MemoryListParams,
  MemoryRecord,
  MemoryStatsWithTrends,
  UpdateMemoryRecordRequest,
} from '@/api/soulbrowser';

import styles from './TasksPage.module.css';

interface TemplatesPaneProps {
  onApplyTemplate?: (record: MemoryRecord) => void;
  onTemplateDeleted?: (record: MemoryRecord) => void;
  onTemplateUpdated?: (record: MemoryRecord) => void;
}

interface FilterValues {
  namespace?: string;
  tag?: string;
  limit?: number;
}

interface CreateTemplateValues {
  namespace: string;
  key: string;
  tags?: string;
  note?: string;
  metadata?: string;
}

interface EditTemplateValues {
  tags?: string;
  note?: string;
  metadata?: string;
}

const DEFAULT_NAMESPACE = 'templates';

export default function TemplatesPane({
  onApplyTemplate,
  onTemplateDeleted,
  onTemplateUpdated,
}: TemplatesPaneProps) {
  const [records, setRecords] = useState<MemoryRecord[]>([]);
  const [loading, setLoading] = useState(false);
  const [creating, setCreating] = useState(false);
  const [deletingId, setDeletingId] = useState<string | null>(null);
  const [filterForm] = Form.useForm<FilterValues>();
  const [createForm] = Form.useForm<CreateTemplateValues>();
  const [editForm] = Form.useForm<EditTemplateValues>();
  const [editingRecord, setEditingRecord] = useState<MemoryRecord | null>(null);
  const [editVisible, setEditVisible] = useState(false);
  const [editLoading, setEditLoading] = useState(false);
  const [stats, setStats] = useState<MemoryStatsWithTrends | null>(null);
  const [statsLoading, setStatsLoading] = useState(false);

  const fetchRecords = useCallback(
    async (values?: FilterValues) => {
      const params = values ?? filterForm.getFieldsValue(true);
      setLoading(true);
      try {
        const normalized: MemoryListParams = {
          namespace: params.namespace?.trim() || undefined,
          tag: params.tag?.trim() || undefined,
          limit: params.limit,
        };
        const items = await soulbrowserAPI.listMemoryRecords(normalized);
        setRecords(items);
      } catch (err) {
        console.error(err);
        message.error('无法加载模板列表');
      } finally {
        setLoading(false);
      }
    },
    [filterForm]
  );

  const fetchStats = useCallback(async () => {
    setStatsLoading(true);
    try {
      const snapshot = await soulbrowserAPI.getMemoryStats();
      setStats(snapshot);
    } catch (err) {
      console.error(err);
      message.warning('无法获取记忆统计');
    } finally {
      setStatsLoading(false);
    }
  }, []);

  useEffect(() => {
    filterForm.setFieldsValue({ namespace: DEFAULT_NAMESPACE, limit: 20 });
    createForm.setFieldsValue({ namespace: DEFAULT_NAMESPACE });
    void fetchRecords({ namespace: DEFAULT_NAMESPACE, limit: 20 });
    void fetchStats();
  }, [createForm, fetchRecords, fetchStats, filterForm]);

  const handleFilterSubmit = (values: FilterValues) => {
    void fetchRecords(values);
  };

  const handleCreateTemplate = async (values: CreateTemplateValues) => {
    const namespace = values.namespace?.trim();
    const key = values.key?.trim();
    if (!namespace || !key) {
      message.warning('命名空间和 Key 必填');
      return;
    }

    let metadata: Record<string, unknown> | undefined;
    if (values.metadata && values.metadata.trim().length > 0) {
      try {
        metadata = JSON.parse(values.metadata);
      } catch (err) {
        console.error(err);
        message.error('Metadata 必须是合法 JSON');
        return;
      }
    }

    const payload: CreateMemoryRecordRequest = {
      namespace,
      key,
      note: values.note?.trim() || undefined,
      tags: values.tags
        ? values.tags
            .split(',')
            .map((tag) => tag.trim())
            .filter((tag) => tag.length > 0)
        : undefined,
      metadata,
    };

    setCreating(true);
    try {
      await soulbrowserAPI.createMemoryRecord(payload);
      message.success('模板已保存');
      createForm.resetFields();
      createForm.setFieldsValue({ namespace });
      void fetchRecords();
      void fetchStats();
    } catch (err) {
      console.error(err);
      message.error('保存模板失败');
    } finally {
      setCreating(false);
    }
  };

  const handleApplyTemplate = (record: MemoryRecord) => {
    onApplyTemplate?.(record);
  };

  const handleDeleteTemplate = useCallback(
    async (record: MemoryRecord) => {
      setDeletingId(record.id);
      try {
        await soulbrowserAPI.deleteMemoryRecord(record.id);
        message.success('模板已删除');
        setRecords((prev) => prev.filter((item) => item.id !== record.id));
        onTemplateDeleted?.(record);
        if (editingRecord?.id === record.id) {
          setEditVisible(false);
          setEditingRecord(null);
          editForm.resetFields();
        }
      } catch (err) {
        console.error(err);
        message.error('删除模板失败');
      } finally {
        setDeletingId(null);
        void fetchRecords();
        void fetchStats();
      }
    },
    [editForm, editingRecord, fetchRecords, fetchStats, onTemplateDeleted]
  );

  const openEditModal = useCallback(
    (record: MemoryRecord) => {
      setEditingRecord(record);
      editForm.setFieldsValue({
        tags: record.tags.join(','),
        note: record.note || '',
        metadata: record.metadata ? JSON.stringify(record.metadata, null, 2) : '',
      });
      setEditVisible(true);
    },
    [editForm]
  );

  const handleUpdateTemplate = useCallback(async () => {
    if (!editingRecord) return;
    try {
      const values = (await editForm.validateFields()) as EditTemplateValues;
      const payload: UpdateMemoryRecordRequest = {};

      if (values.tags !== undefined) {
        const parsedTags = values.tags
          ? values.tags
              .split(',')
              .map((tag) => tag.trim())
              .filter((tag) => tag.length > 0)
          : [];
        payload.tags = parsedTags;
      }
      if (values.note !== undefined) {
        const trimmed = values.note.trim();
        payload.note = trimmed.length ? trimmed : null;
      }
      if (values.metadata !== undefined) {
        if (!values.metadata.trim()) {
          payload.metadata = null;
        } else {
          payload.metadata = JSON.parse(values.metadata);
        }
      }

      if (!('tags' in payload) && !('note' in payload) && !('metadata' in payload)) {
        message.info('无更新内容');
        return;
      }

      setEditLoading(true);
      const updated = await soulbrowserAPI.updateMemoryRecord(editingRecord.id, payload);
      setRecords((prev) => prev.map((item) => (item.id === updated.id ? updated : item)));
      onTemplateUpdated?.(updated);
      message.success('模板已更新');
      setEditVisible(false);
      setEditingRecord(null);
      editForm.resetFields();
      void fetchStats();
    } catch (err: any) {
      if (err?.errorFields) {
        return;
      }
      if (err instanceof SyntaxError) {
        message.error('Metadata 必须是合法 JSON');
        return;
      }
      console.error(err);
      message.error('更新模板失败');
      return;
    } finally {
      setEditLoading(false);
    }
  }, [editForm, editingRecord, onTemplateUpdated]);

  return (
    <div className={styles.templatesSection}>
      <div>
        <Typography.Title level={4}>记忆 / 模板</Typography.Title>
        <Typography.Text type="secondary">
          保存常用提示词和上下文，快速填充到任务创建表单
        </Typography.Text>
      </div>

      <div style={{ marginTop: 12 }}>
        <Spin spinning={statsLoading}>
          <Descriptions size="small" bordered column={3} labelStyle={{ fontWeight: 500 }}>
            <Descriptions.Item label="查询次数">
              {stats?.total_queries ?? 0}
            </Descriptions.Item>
            <Descriptions.Item label="命中次数">
              {stats?.hit_queries ?? 0}
            </Descriptions.Item>
            <Descriptions.Item label="命中率">
              {stats ? `${(stats.hit_rate * 100).toFixed(1)}%` : '0.0%'}
            </Descriptions.Item>
            <Descriptions.Item label="存储次数">
              {stats?.stored_records ?? 0}
            </Descriptions.Item>
            <Descriptions.Item label="删除次数">
              {stats?.deleted_records ?? 0}
            </Descriptions.Item>
            <Descriptions.Item label="当前记录">
              {stats?.current_records ?? 0}
            </Descriptions.Item>
            <Descriptions.Item label="模板使用">
              {stats?.template_uses ?? 0}
            </Descriptions.Item>
            <Descriptions.Item label="模板成功">
              {stats?.template_successes ?? 0}
            </Descriptions.Item>
            <Descriptions.Item label="模板成功率">
              {stats ? `${(stats.template_success_rate * 100).toFixed(1)}%` : '0.0%'}
            </Descriptions.Item>
          </Descriptions>
        </Spin>
      </div>

      <Form
        layout="inline"
        form={filterForm}
        className={styles.templateFilterForm}
        onFinish={handleFilterSubmit}
      >
        <Form.Item label="命名空间" name="namespace">
          <Input placeholder="例如 templates" allowClear />
        </Form.Item>
        <Form.Item label="标签" name="tag">
          <Input placeholder="可选" allowClear />
        </Form.Item>
        <Form.Item label="数量" name="limit">
          <InputNumber min={1} max={100} placeholder="20" />
        </Form.Item>
        <Form.Item>
          <Space>
            <Button type="primary" htmlType="submit">
              筛选
            </Button>
            <Button icon={<ReloadOutlined />} onClick={() => fetchRecords()}>
              刷新
            </Button>
          </Space>
        </Form.Item>
      </Form>

      <div className={styles.templatesContent}>
        <div className={styles.templateListWrapper}>
          {records.length ? (
            <List
              dataSource={records}
              loading={loading}
              renderItem={(item) => (
                <List.Item key={item.id} className={styles.templateCard}>
                  <Space direction="vertical" size={6} className={styles.templateCardBody}>
                    <Space size={8} wrap>
                      <Typography.Text strong>{item.key}</Typography.Text>
                      <Tag color="geekblue">{item.namespace}</Tag>
                      <Typography.Text type="secondary">
                        {new Date(item.created_at).toLocaleString()}
                      </Typography.Text>
                    </Space>
                    {item.tags && item.tags.length > 0 && (
                      <div className={styles.templateTags}>
                        {item.tags.map((tag) => (
                          <Tag key={`${item.id}-${tag}`}>{tag}</Tag>
                        ))}
                      </div>
                    )}
                    {item.note && <Typography.Text>{item.note}</Typography.Text>}
                    {item.metadata && (
                      <pre className={styles.templateMetadata}>
                        {JSON.stringify(item.metadata, null, 2)}
                      </pre>
                    )}
                    <Space size={8} wrap>
                      <Tag color="purple">使用 {item.use_count}</Tag>
                      <Tag color="green">成功 {item.success_count}</Tag>
                      <Typography.Text type="secondary">
                        最近使用：
                        {item.last_used_at
                          ? dayjs(item.last_used_at).format('YYYY-MM-DD HH:mm')
                          : '未使用'}
                      </Typography.Text>
                    </Space>
                    <div className={styles.templateActions}>
                      <Space size="small">
                        <Button
                          type="primary"
                          icon={<PlayCircleOutlined />}
                          onClick={() => handleApplyTemplate(item)}
                        >
                          填充任务
                        </Button>
                        <Button
                          icon={<EditOutlined />}
                          onClick={() => openEditModal(item)}
                        >
                          编辑
                        </Button>
                        <Popconfirm
                          title="删除模板"
                          description="确认删除该模板？"
                          okText="删除"
                          cancelText="取消"
                          onConfirm={() => handleDeleteTemplate(item)}
                        >
                          <Button
                            icon={<DeleteOutlined />}
                            danger
                            loading={deletingId === item.id}
                          >
                            删除
                          </Button>
                        </Popconfirm>
                      </Space>
                    </div>
                  </Space>
                </List.Item>
              )}
            />
          ) : (
            <div className={styles.templateEmptyState}>
              {loading ? (
                <Typography.Text type="secondary">加载中…</Typography.Text>
              ) : (
                <Empty description="暂无模板" />
              )}
            </div>
          )}
        </div>

        <div className={styles.templateCreateCard}>
          <Typography.Title level={5}>保存新模板</Typography.Title>
          <Form layout="vertical" form={createForm} onFinish={handleCreateTemplate}>
            <Form.Item
              label="命名空间"
              name="namespace"
              rules={[{ required: true, message: '请输入命名空间' }]}
            >
              <Input placeholder="例如 templates" allowClear />
            </Form.Item>
            <Form.Item
              label="Key"
              name="key"
              rules={[{ required: true, message: '请输入 Key' }]}
            >
              <Input placeholder="关键字" allowClear />
            </Form.Item>
            <Form.Item label="标签" name="tags" tooltip="用逗号分隔">
              <Input placeholder="tag1,tag2" allowClear />
            </Form.Item>
            <Form.Item label="备注" name="note">
              <Input.TextArea rows={2} placeholder="说明或适用场景" allowClear />
            </Form.Item>
            <Form.Item label="Metadata JSON" name="metadata">
              <Input.TextArea rows={4} placeholder='{"prompt":"..."}' allowClear />
            </Form.Item>
            <Button
              type="primary"
              htmlType="submit"
              icon={<SaveOutlined />}
              loading={creating}
              block
            >
              保存模板
            </Button>
          </Form>
        </div>
      </div>

      <Modal
        title={editingRecord ? `编辑模板：${editingRecord.key}` : '编辑模板'}
        open={editVisible}
        onCancel={() => {
          setEditVisible(false);
          setEditingRecord(null);
          editForm.resetFields();
        }}
        onOk={handleUpdateTemplate}
        confirmLoading={editLoading}
        destroyOnClose
      >
        <Form layout="vertical" form={editForm}>
          <Form.Item label="标签" name="tags" tooltip="用逗号分隔">
            <Input placeholder="tag1,tag2" allowClear />
          </Form.Item>
          <Form.Item label="备注" name="note">
            <Input.TextArea rows={3} placeholder="说明或适用场景" allowClear />
          </Form.Item>
          <Form.Item label="Metadata JSON" name="metadata">
            <Input.TextArea rows={4} placeholder='{"prompt":"..."}' allowClear />
          </Form.Item>
        </Form>
      </Modal>
    </div>
  );
}
