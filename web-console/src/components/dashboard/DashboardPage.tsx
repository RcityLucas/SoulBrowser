import { useEffect } from 'react';
import { Card, Row, Col, Statistic } from 'antd';
import { ArrowUpOutlined, ArrowDownOutlined } from '@ant-design/icons';
import ReactECharts from 'echarts-for-react';
import { useMetricsStore } from '@/stores/metricsStore';
import { formatDuration } from '@/utils/format';
import styles from './DashboardPage.module.css';

export default function DashboardPage() {
  const { report, fetchMetrics } = useMetricsStore();

  useEffect(() => {
    fetchMetrics();
    const interval = setInterval(fetchMetrics, 30000); // Refresh every 30 seconds
    return () => clearInterval(interval);
  }, [fetchMetrics]);

  const successRateOption = {
    title: { text: '任务成功率趋势', textStyle: { color: '#fff' } },
    backgroundColor: 'transparent',
    tooltip: { trigger: 'axis' },
    xAxis: {
      type: 'category',
      data: report?.timeSeries.successRate.map((d) => new Date(d.timestamp).toLocaleTimeString()) || [],
      axisLine: { lineStyle: { color: '#8c8c8c' } },
    },
    yAxis: {
      type: 'value',
      max: 100,
      axisLine: { lineStyle: { color: '#8c8c8c' } },
    },
    series: [
      {
        data: report?.timeSeries.successRate.map((d) => d.value) || [],
        type: 'line',
        smooth: true,
        itemStyle: { color: '#52c41a' },
      },
    ],
  };

  const taskCountOption = {
    title: { text: '任务数量统计', textStyle: { color: '#fff' } },
    backgroundColor: 'transparent',
    tooltip: { trigger: 'axis' },
    xAxis: {
      type: 'category',
      data: report?.timeSeries.taskCount.map((d) => new Date(d.timestamp).toLocaleTimeString()) || [],
      axisLine: { lineStyle: { color: '#8c8c8c' } },
    },
    yAxis: {
      type: 'value',
      axisLine: { lineStyle: { color: '#8c8c8c' } },
    },
    series: [
      {
        data: report?.timeSeries.taskCount.map((d) => d.value) || [],
        type: 'bar',
        itemStyle: { color: '#1890ff' },
      },
    ],
  };

  return (
    <div className={styles.dashboardPage}>
      <Row gutter={[24, 24]}>
        <Col span={6}>
          <Card className={styles.card}>
            <Statistic
              title="总任务数"
              value={report?.summary.totalTasks || 0}
              valueStyle={{ color: '#fff' }}
            />
          </Card>
        </Col>
        <Col span={6}>
          <Card className={styles.card}>
            <Statistic
              title="成功率"
              value={report?.summary.successRate || 0}
              precision={1}
              suffix="%"
              valueStyle={{ color: '#52c41a' }}
              prefix={<ArrowUpOutlined />}
            />
          </Card>
        </Col>
        <Col span={6}>
          <Card className={styles.card}>
            <Statistic
              title="平均耗时"
              value={formatDuration(report?.summary.avgDuration || 0)}
              valueStyle={{ color: '#fff' }}
            />
          </Card>
        </Col>
        <Col span={6}>
          <Card className={styles.card}>
            <Statistic
              title="失败任务"
              value={report?.summary.failedTasks || 0}
              valueStyle={{ color: '#ff4d4f' }}
              prefix={<ArrowDownOutlined />}
            />
          </Card>
        </Col>
      </Row>

      <Row gutter={[24, 24]} style={{ marginTop: 24 }}>
        <Col span={12}>
          <Card className={styles.card}>
            <ReactECharts option={successRateOption} style={{ height: 300 }} />
          </Card>
        </Col>
        <Col span={12}>
          <Card className={styles.card}>
            <ReactECharts option={taskCountOption} style={{ height: 300 }} />
          </Card>
        </Col>
      </Row>
    </div>
  );
}
