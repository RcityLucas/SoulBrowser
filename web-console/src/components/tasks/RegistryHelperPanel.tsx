import { useEffect, useState } from 'react';
import {
  Button,
  Drawer,
  Form,
  Input,
  List,
  Modal,
  Popconfirm,
  Select,
  Space,
  Spin,
  Switch,
  Tag,
  Typography,
  message,
} from 'antd';
import type { RegistryHelper, RegistryHelperStep } from '@/api/soulbrowser';
import soulbrowserAPI from '@/api/soulbrowser';

interface RegistryHelperPanelProps {
  pluginId: string;
  visible: boolean;
  onClose: () => void;
}

type HelperEditorMode = 'create' | 'edit';

interface HelperFormValues {
  id: string;
  pattern: string;
  description?: string;
  prompt?: string;
  auto_insert?: boolean;
  blockers?: string[];
  stepsContent: string;
  conditionsIncludes?: string[];
  conditionsExcludes?: string[];
}

const { TextArea } = Input;

export default function RegistryHelperPanel({ pluginId, visible, onClose }: RegistryHelperPanelProps) {
  const [loading, setLoading] = useState(false);
  const [helpers, setHelpers] = useState<RegistryHelper[]>([]);
  const [scaffoldLoading, setScaffoldLoading] = useState(false);
  const [downloadLoading, setDownloadLoading] = useState(false);
  const [editorVisible, setEditorVisible] = useState(false);
  const [editorMode, setEditorMode] = useState<HelperEditorMode>('create');
  const [editorSaving, setEditorSaving] = useState(false);
  const [currentHelperId, setCurrentHelperId] = useState<string | null>(null);
  const [form] = Form.useForm<HelperFormValues>();

  const defaultStepsTemplate: RegistryHelperStep[] = [
    {
      title: 'Describe helper step',
      detail: 'Explain what this helper action should do',
      tool: { type: 'click_css', selector: '#cta-button' },
    },
  ];

  const serializeStepsContent = (helper?: RegistryHelper) => {
    if (!helper) {
      return JSON.stringify(defaultStepsTemplate, null, 2);
    }
    const payload = helper.steps ?? helper.step;
    if (!payload) {
      return JSON.stringify(defaultStepsTemplate, null, 2);
    }
    return JSON.stringify(payload, null, 2);
  };

  const openEditor = (mode: HelperEditorMode, helper: RegistryHelper) => {
    setEditorMode(mode);
    setEditorVisible(true);
    setCurrentHelperId(mode === 'edit' ? helper.id : null);
    form.setFieldsValue({
      id: helper.id,
      pattern: helper.pattern,
      description: helper.description ?? '',
      prompt: helper.prompt ?? '',
      auto_insert: helper.auto_insert ?? false,
      blockers: helper.blockers ?? [],
      stepsContent: serializeStepsContent(helper),
      conditionsIncludes: helper.conditions?.url_includes ?? [],
      conditionsExcludes: helper.conditions?.url_excludes ?? [],
    });
  };

  useEffect(() => {
    if (!visible) {
      return;
    }
    setLoading(true);
    soulbrowserAPI
      .listPluginHelpers(pluginId)
      .then((items) => {
        setHelpers(items);
      })
      .catch((err) => {
        console.error(err);
        message.error(err.message || '加载 registry helpers 失败');
      })
      .finally(() => setLoading(false));
  }, [pluginId, visible]);

  const hasSteps = (helper?: RegistryHelper | null) => {
    if (!helper) return false;
    const steps = helper.steps ?? [];
    return steps.length > 0 || Boolean(helper.step);
  };

  const handleCreateHelper = async () => {
    setScaffoldLoading(true);
    try {
      const helperTemplate = await soulbrowserAPI.scaffoldPluginHelper(pluginId, {
        id: `${pluginId}_helper_${helpers.length + 1}`,
        pattern: 'https://example.com',
      });
      openEditor('create', helperTemplate);
    } catch (err) {
      console.error(err);
      message.error((err as Error)?.message || '无法生成 scaffold');
    } finally {
      setScaffoldLoading(false);
    }
  };

  const handleDownloadScaffold = async () => {
    setDownloadLoading(true);
    try {
      const helperTemplate = await soulbrowserAPI.scaffoldPluginHelper(pluginId, {
        id: `${pluginId}_helper_${helpers.length + 1}`,
        pattern: 'https://example.com',
      });
      const blob = new Blob([JSON.stringify(helperTemplate, null, 2)], {
        type: 'application/json',
      });
      const url = URL.createObjectURL(blob);
      const link = document.createElement('a');
      link.href = url;
      link.download = `${pluginId}_helper.json`;
      document.body.appendChild(link);
      link.click();
      link.remove();
      URL.revokeObjectURL(url);
    } catch (err) {
      console.error(err);
      message.error((err as Error)?.message || '无法生成 scaffold');
    } finally {
      setDownloadLoading(false);
    }
  };

  const handleEditHelper = (helper: RegistryHelper) => {
    openEditor('edit', helper);
  };

  const handleDeleteHelper = async (helperId: string) => {
    try {
      await soulbrowserAPI.deletePluginHelper(pluginId, helperId);
      setHelpers((prev) => prev.filter((item) => item.id !== helperId));
      message.success('Helper 已删除');
    } catch (err) {
      console.error(err);
      message.error((err as Error)?.message || '删除 helper 失败');
    }
  };

  const handleEditorCancel = () => {
    setEditorVisible(false);
    setCurrentHelperId(null);
    form.resetFields();
  };

  const handleEditorSubmit = async () => {
    try {
      const values = await form.validateFields();
      const stepsRaw = values.stepsContent?.trim();
      let stepsPayload: unknown;
      if (stepsRaw) {
        try {
          stepsPayload = JSON.parse(stepsRaw);
        } catch (err) {
          message.error('步骤 JSON 解析失败');
          return;
        }
      }
      if (!values.id?.trim()) {
        message.error('Helper ID 不能为空');
        return;
      }
      if (!values.pattern?.trim()) {
        message.error('Pattern 不能为空');
        return;
      }
      const blockers = (values.blockers ?? [])
        .map((item) => item.trim())
        .filter((item) => item.length > 0);
      const helperPayload: RegistryHelper = {
        id: values.id.trim(),
        pattern: values.pattern.trim(),
        description: values.description?.trim() || undefined,
        prompt: values.prompt?.trim() || undefined,
        auto_insert: Boolean(values.auto_insert),
        blockers: blockers.length > 0 ? blockers : undefined,
      };
      if (stepsPayload) {
        if (Array.isArray(stepsPayload)) {
          helperPayload.steps = stepsPayload as RegistryHelperStep[];
          helperPayload.step = undefined;
        } else if (typeof stepsPayload === 'object') {
          helperPayload.step = stepsPayload as RegistryHelperStep;
          helperPayload.steps = undefined;
        }
      }
      const includes = (values.conditionsIncludes ?? [])
        .map((item) => item.trim())
        .filter((item) => item.length > 0);
      const excludes = (values.conditionsExcludes ?? [])
        .map((item) => item.trim())
        .filter((item) => item.length > 0);
      if (includes.length > 0 || excludes.length > 0) {
        helperPayload.conditions = {
          url_includes: includes.length > 0 ? includes : undefined,
          url_excludes: excludes.length > 0 ? excludes : undefined,
        };
      }
      setEditorSaving(true);
      const savedHelper =
        editorMode === 'create'
          ? await soulbrowserAPI.createPluginHelper(pluginId, helperPayload)
          : await soulbrowserAPI.updatePluginHelper(
              pluginId,
              currentHelperId ?? helperPayload.id,
              helperPayload,
            );
      setHelpers((prev) => {
        if (editorMode === 'create') {
          return [savedHelper, ...prev];
        }
        const compareId = currentHelperId ?? savedHelper.id;
        return prev.map((item) => (item.id === compareId ? savedHelper : item));
      });
      message.success(editorMode === 'create' ? 'Helper 已创建' : 'Helper 已更新');
      handleEditorCancel();
    } catch (err: any) {
      if (err?.errorFields) {
        return;
      }
      console.error(err);
      message.error(err?.message || '保存 helper 失败');
    } finally {
      setEditorSaving(false);
    }
  };

  return (
    <>
      <Drawer
        title={`Registry Helpers (${pluginId})`}
        placement="right"
        width={420}
        open={visible}
        onClose={onClose}
      >
        <Space direction="vertical" style={{ width: '100%' }}>
          <Space style={{ width: '100%', justifyContent: 'space-between' }}>
            <Button type="primary" onClick={handleCreateHelper} loading={scaffoldLoading}>
              新建 Helper
            </Button>
            <Button onClick={handleDownloadScaffold} loading={downloadLoading}>
              下载 Scaffold
            </Button>
          </Space>
          {loading ? (
            <Spin />
          ) : (
            <List
              itemLayout="vertical"
              dataSource={helpers}
              locale={{ emptyText: '暂无 helper' }}
              renderItem={(helper) => (
                <List.Item
                  key={helper.id}
                  actions={[
                    hasSteps(helper) ? (
                      <Tag color="green">steps</Tag>
                    ) : (
                      <Tag color="red">missing steps</Tag>
                    ),
                    <Button type="link" size="small" onClick={() => handleEditHelper(helper)}>
                      编辑
                    </Button>,
                    <Popconfirm
                      title="确定删除该 helper?"
                      okText="删除"
                      cancelText="取消"
                      onConfirm={() => handleDeleteHelper(helper.id)}
                    >
                      <Button type="link" danger size="small">
                        删除
                      </Button>
                    </Popconfirm>,
                  ]}
                >
                  <List.Item.Meta
                    title={helper.id}
                    description={
                      <Space direction="vertical">
                        <Typography.Text type="secondary">{helper.pattern}</Typography.Text>
                        {helper.description && <Typography.Text>{helper.description}</Typography.Text>}
                        {helper.blockers && helper.blockers.length > 0 && (
                          <Typography.Text type="secondary">
                            Blockers: {helper.blockers.join(', ')}
                          </Typography.Text>
                        )}
                      </Space>
                    }
                  />
                  {helper.prompt && (
                    <Typography.Paragraph ellipsis={{ rows: 2 }}>
                      {helper.prompt}
                    </Typography.Paragraph>
                  )}
                </List.Item>
              )}
            />
          )}
        </Space>
      </Drawer>
      <Modal
        title={editorMode === 'create' ? '新建 Helper' : `编辑 Helper ${currentHelperId ?? ''}`}
        open={editorVisible}
        onCancel={handleEditorCancel}
        onOk={handleEditorSubmit}
        confirmLoading={editorSaving}
        width={720}
        okText="保存"
        cancelText="取消"
      >
        <Form form={form} layout="vertical" preserve={false}>
          <Form.Item
            label="Helper ID"
            name="id"
            rules={[{ required: true, message: '请输入 helper ID' }]}
          >
            <Input placeholder="plugin_helper_id" disabled={editorMode === 'edit'} />
          </Form.Item>
          <Form.Item
            label="URL Pattern"
            name="pattern"
            rules={[{ required: true, message: '请输入 URL pattern' }]}
          >
            <Input placeholder="https://example.com/*" />
          </Form.Item>
          <Form.Item label="描述" name="description">
            <Input placeholder="描述 helper 处理的场景" />
          </Form.Item>
          <Form.Item label="Prompt" name="prompt">
            <TextArea rows={3} placeholder="可选的提示内容" />
          </Form.Item>
          <Form.Item label="自动插入" name="auto_insert" valuePropName="checked">
            <Switch />
          </Form.Item>
          <Form.Item label="Blockers" name="blockers">
            <Select mode="tags" placeholder="输入 blocker 标签" />
          </Form.Item>
          <Form.Item
            label="步骤 JSON"
            name="stepsContent"
            rules={[{ required: true, message: '请填写步骤 JSON' }]}
          >
            <TextArea rows={8} placeholder="[{...steps...}] 或单个 step 对象" />
          </Form.Item>
          <Form.Item label="URL 包含" name="conditionsIncludes">
            <Select mode="tags" placeholder="可选，限定 URL 包含片段" />
          </Form.Item>
          <Form.Item label="URL 排除" name="conditionsExcludes">
            <Select mode="tags" placeholder="可选，限定 URL 排除片段" />
          </Form.Item>
        </Form>
      </Modal>
    </>
  );
}
