import { useState } from 'react';
import {
  Card,
  Form,
  Input,
  Select,
  Button,
  Checkbox,
  Space,
  Alert,
  Descriptions,
  Tag,
  Image,
  Collapse,
  Spin,
} from 'antd';
import { PlayCircleOutlined, CodeOutlined, FileImageOutlined } from '@ant-design/icons';
import { soulbrowserAPI, type PerceiveRequest, type PerceiveResponse } from '@/api/soulbrowser';
import styles from './PerceivePage.module.css';

const { Panel } = Collapse;

export default function PerceivePage() {
  const [form] = Form.useForm();
  const [loading, setLoading] = useState(false);
  const [result, setResult] = useState<PerceiveResponse | null>(null);
  const [error, setError] = useState<string | null>(null);

  const handleSubmit = async (values: any) => {
    setLoading(true);
    setError(null);
    setResult(null);

    try {
      const request: PerceiveRequest = {
        url: values.url,
        mode: values.mode,
        screenshot: values.screenshot,
        insights: values.insights,
      };

      if (values.mode === 'custom') {
        request.structural = values.structural;
        request.visual = values.visual;
        request.semantic = values.semantic;
      }

      const response = await soulbrowserAPI.perceive(request);
      setResult(response);

      if (!response.success) {
        setError('Perception failed. Check stderr output below.');
      }
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to execute perception');
    } finally {
      setLoading(false);
    }
  };

  const renderStructuralData = () => {
    if (!result?.perception?.structural) return null;

    const data = result.perception.structural;
    return (
      <Card title="🏗️ Structural Perception" size="small" className={styles.resultCard}>
        <Descriptions column={2} size="small">
          <Descriptions.Item label="DOM Nodes">{data.dom_node_count}</Descriptions.Item>
          <Descriptions.Item label="Forms">{data.form_count}</Descriptions.Item>
          <Descriptions.Item label="Interactive Elements">
            {data.interactive_count}
          </Descriptions.Item>
          <Descriptions.Item label="Text Nodes">{data.text_node_count}</Descriptions.Item>
        </Descriptions>
      </Card>
    );
  };

  const renderVisualData = () => {
    if (!result?.perception?.visual) return null;

    const data = result.perception.visual;
    return (
      <Card title="🎨 Visual Perception" size="small" className={styles.resultCard}>
        <Descriptions column={2} size="small">
          <Descriptions.Item label="Viewport">
            {data.viewport_width} × {data.viewport_height}
          </Descriptions.Item>
          <Descriptions.Item label="Dominant Colors">
            <Space>
              {data.dominant_colors?.map((color, index) => (
                <Tag key={index} color={color}>
                  {color}
                </Tag>
              ))}
            </Space>
          </Descriptions.Item>
        </Descriptions>
      </Card>
    );
  };

  const renderSemanticData = () => {
    if (!result?.perception?.semantic) return null;

    const data = result.perception.semantic;
    return (
      <Card title="🧠 Semantic Perception" size="small" className={styles.resultCard}>
        <Descriptions column={1} size="small">
          <Descriptions.Item label="Content Type">{data.content_type}</Descriptions.Item>
          <Descriptions.Item label="Main Heading">{data.main_heading}</Descriptions.Item>
          <Descriptions.Item label="Language">{data.language}</Descriptions.Item>
        </Descriptions>
      </Card>
    );
  };

  const renderInsights = () => {
    if (!result?.perception?.insights || result.perception.insights.length === 0) return null;

    return (
      <Card title="💡 Insights" size="small" className={styles.resultCard}>
        <Space direction="vertical" style={{ width: '100%' }}>
          {result.perception.insights.map((insight, index) => (
            <Alert
              key={index}
              message={insight.type}
              description={insight.message}
              type={insight.severity === 'warning' ? 'warning' : 'info'}
              showIcon
            />
          ))}
        </Space>
      </Card>
    );
  };

  return (
    <div className={styles.perceivePage}>
      <Card title="🔍 Multi-Modal Page Perception" className={styles.card}>
        <Form
          form={form}
          layout="vertical"
          initialValues={{
            url: 'https://example.com',
            mode: 'all',
            screenshot: true,
            insights: true,
          }}
          onFinish={handleSubmit}
        >
          <Form.Item
            label="Target URL"
            name="url"
            rules={[{ required: true, message: 'Please enter a URL' }]}
          >
            <Input placeholder="https://example.com" size="large" />
          </Form.Item>

          <Form.Item label="Perception Mode" name="mode">
            <Select size="large">
              <Select.Option value="all">All (Structural + Visual + Semantic)</Select.Option>
              <Select.Option value="structural">Structural Only</Select.Option>
              <Select.Option value="visual">Visual Only</Select.Option>
              <Select.Option value="semantic">Semantic Only</Select.Option>
              <Select.Option value="custom">Custom Selection</Select.Option>
            </Select>
          </Form.Item>

          <Form.Item noStyle shouldUpdate={(prev, curr) => prev.mode !== curr.mode}>
            {({ getFieldValue }) =>
              getFieldValue('mode') === 'custom' ? (
                <Space direction="vertical" style={{ width: '100%', marginBottom: 16 }}>
                  <Form.Item name="structural" valuePropName="checked" noStyle>
                    <Checkbox>Structural Perception</Checkbox>
                  </Form.Item>
                  <Form.Item name="visual" valuePropName="checked" noStyle>
                    <Checkbox>Visual Perception</Checkbox>
                  </Form.Item>
                  <Form.Item name="semantic" valuePropName="checked" noStyle>
                    <Checkbox>Semantic Perception</Checkbox>
                  </Form.Item>
                </Space>
              ) : null
            }
          </Form.Item>

          <Space>
            <Form.Item name="screenshot" valuePropName="checked" noStyle>
              <Checkbox>Capture Screenshot</Checkbox>
            </Form.Item>
            <Form.Item name="insights" valuePropName="checked" noStyle>
              <Checkbox>Generate Insights</Checkbox>
            </Form.Item>
          </Space>

          <Form.Item style={{ marginTop: 24 }}>
            <Button
              type="primary"
              htmlType="submit"
              icon={<PlayCircleOutlined />}
              loading={loading}
              size="large"
              block
            >
              Run Perception
            </Button>
          </Form.Item>
        </Form>

        {error && (
          <Alert message="Error" description={error} type="error" showIcon closable style={{ marginTop: 16 }} />
        )}

        {loading && (
          <div style={{ textAlign: 'center', padding: 40 }}>
            <Spin size="large" tip="Running perception..." />
          </div>
        )}

        {result && (
          <div className={styles.results}>
            <Alert
              message={result.success ? 'Success' : 'Failed'}
              type={result.success ? 'success' : 'error'}
              showIcon
              style={{ marginBottom: 16 }}
            />

            <Space direction="vertical" style={{ width: '100%' }} size="large">
              {renderStructuralData()}
              {renderVisualData()}
              {renderSemanticData()}
              {renderInsights()}

              {result.screenshot_base64 && (
                <Card
                  title={
                    <>
                      <FileImageOutlined /> Screenshot
                    </>
                  }
                  size="small"
                  className={styles.resultCard}
                >
                  <Image
                    src={`data:image/png;base64,${result.screenshot_base64}`}
                    alt="Page Screenshot"
                    style={{ maxWidth: '100%' }}
                  />
                </Card>
              )}

              <Collapse>
                <Panel header={<><CodeOutlined /> Raw JSON Payload</>} key="json">
                  <pre className={styles.jsonOutput}>
                    {JSON.stringify(result.perception, null, 2)}
                  </pre>
                </Panel>

                {result.stdout && (
                  <Panel header="📝 STDOUT" key="stdout">
                    <pre className={styles.logOutput}>{result.stdout}</pre>
                  </Panel>
                )}

                {result.stderr && (
                  <Panel header="⚠️ STDERR" key="stderr">
                    <pre className={styles.logOutput}>{result.stderr}</pre>
                  </Panel>
                )}
              </Collapse>
            </Space>
          </div>
        )}
      </Card>
    </div>
  );
}
