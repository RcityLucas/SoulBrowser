use agent_core::{AgentRequest, ConversationRole, ConversationTurn};
use serde_json::Value;
use url::form_urlencoded;

use crate::agent::{FlowExecutionReport, StepExecutionStatus};
use crate::intent::update_todo_snapshot;

const RECENT_HISTORY_LIMIT: usize = 6;
const HISTORY_SUMMARY_LIMIT: usize = 240;

/// Enrich the agent request with failure context so LLM planners can replan intelligently.
pub fn augment_request_for_replan(
    request: &AgentRequest,
    report: &FlowExecutionReport,
    attempt: u32,
    observation_summary: Option<&str>,
    blocker_kind: Option<&str>,
    agent_history_prompt: Option<&str>,
) -> Option<(AgentRequest, String)> {
    let failure_step = report
        .steps
        .iter()
        .rev()
        .find(|step| matches!(step.status, StepExecutionStatus::Failed));

    let mut next_request = request.clone();
    let mut failure_summary = if let Some(step) = failure_step {
        let error = step.error.as_deref().unwrap_or("unknown error");
        format!(
            "Execution attempt {} failed at step '{}' after {} attempt(s). Error: {}.",
            attempt + 1,
            step.title,
            step.attempts,
            error
        )
    } else {
        format!(
            "Execution attempt {} failed for unspecified reasons.",
            attempt + 1
        )
    };

    let prompt = format!(
        "{} Please generate an alternative plan that avoids this failure while still achieving the goal.",
        failure_summary
    );
    next_request.push_turn(ConversationTurn::new(ConversationRole::System, prompt));

    if let Some(summary) = observation_summary {
        let note = format!("Latest observation summary: {summary}");
        next_request.push_turn(ConversationTurn::new(
            ConversationRole::System,
            note.clone(),
        ));
        failure_summary.push_str(&format!(" Latest observation summary: {summary}."));
    }

    if let Some(evaluation) = latest_evaluation_summary(report) {
        let note = format!("Latest evaluation: {evaluation}");
        next_request.push_turn(ConversationTurn::new(
            ConversationRole::System,
            note.clone(),
        ));
        next_request.metadata.insert(
            "latest_evaluation_summary".to_string(),
            Value::String(evaluation.clone()),
        );
        failure_summary.push_str(&format!(" Latest evaluation: {evaluation}."));
    } else {
        next_request.metadata.remove("latest_evaluation_summary");
    }

    if let Some(kind) = blocker_kind {
        apply_blocker_guidance(kind, &mut next_request);
        next_request.metadata.insert(
            "registry_blocker_kind".to_string(),
            Value::String(kind.to_string()),
        );
    } else {
        next_request.metadata.remove("registry_blocker_kind");
    }

    let history_block = agent_history_prompt
        .map(|value| value.to_string())
        .or_else(|| recent_step_history(report, RECENT_HISTORY_LIMIT));
    if let Some(history) = history_block {
        next_request.push_turn(ConversationTurn::new(
            ConversationRole::System,
            history.clone(),
        ));
        next_request
            .metadata
            .insert("agent_history_prompt".to_string(), Value::String(history));
    } else {
        next_request.metadata.remove("agent_history_prompt");
    }

    next_request.push_turn(ConversationTurn::new(
        ConversationRole::User,
        "Please suggest a revised plan that can succeed.".to_string(),
    ));
    update_todo_snapshot(&mut next_request);

    Some((next_request, failure_summary))
}

fn apply_blocker_guidance(kind: &str, request: &mut AgentRequest) {
    let guidance = match kind {
        "page_not_found" => {
            ensure_target_site_hint(request, "https://quote.eastmoney.com");
            set_search_terms(
                request,
                vec![
                    format!("东方财富 {}", goal_keyword(request)),
                    format!("新浪财经 {}", goal_keyword(request)),
                ],
            );
            let search_url = search_hint(&request.goal);
            format!(
                "上一轮打开的页面返回 404/NotFound。请改用东方财富等可信行情页面（如 https://quote.eastmoney.com/ 一类的金属报价入口），或先使用搜索引擎（例如 {search_url} ）查找包含目标金属关键词的页面，再继续解析与交付。"
            )
        }
        "quote_fetch_failed" => {
            ensure_target_site_hint(request, "https://quote.eastmoney.com");
            set_search_terms(
                request,
                vec![
                    format!("新浪财经 {}", goal_keyword(request)),
                    format!("东方财富 {} 行情", goal_keyword(request)),
                ],
            );
            let search_url = search_hint(&request.goal);
            format!(
                "行情采集步骤失败，所有配置的数据源都无法获取数据。请改用其他公开行情站点（例如新浪财经）或使用搜索入口 ({search_url}) 重新定位可访问的报价页面，然后再继续解析与交付。"
            )
        }
        "search_no_results" => {
            let fallback = search_hint(&request.goal);
            set_search_terms(
                request,
                vec![
                    format!("{} 行情", goal_keyword(request)),
                    format!("东方财富 {}", goal_keyword(request)),
                    format!("新浪财经 {}", goal_keyword(request)),
                ],
            );
            format!(
                "搜索结果区域未能加载——可能是关键词过于狭窄、需要切换引擎或查看 site: 限定。请调整搜索词（或改用 {fallback} 进行泛搜），确认结果区域已经展示后再继续点击/解析。"
            )
        }
        "popup_unclosed" => "上一轮存在弹窗/浮层阻挡流程。请在执行下一次交互前优先调用 `browser.close-modal` 或 `browser.send-esc`，确保主要内容区域重新可见。".to_string(),
        "url_mismatch" => "上一轮观察到的 URL 与预期不符。请在执行解析前验证目标页面的域名/路径，并插入 data.validate-target 来阻止偏离。".to_string(),
        "weather_results_missing" => "天气入口仍在加载百度首页。请等待天气组件渲染或改为直接打开天气搜索结果。".to_string(),
        "access_blocked" => "站点返回 403/Access Denied。请考虑换用其他公开行情源或通过搜索选择无需登录的页面。".to_string(),
        "target_keywords_missing" => {
            let search_url = search_hint(&request.goal);
            format!(
                "页面缺少目标关键词。请调整搜索词（例如 {search_url} ）或直接打开包含“东方财富 + 品种”的具体行情页后再解析。"
            )
        }
        "target_validation_failed" => {
            let search_url = search_hint(&request.goal);
            format!(
                "目标页面校验失败。请改用可信数据源（东方财富/新浪财经）并确认关键词出现，再执行解析。可先使用 {search_url} 搜索并验证页面内容。"
            )
        }
        _ => format!(
            "Blocker '{kind}' was observed during the last execution. Adjust the new plan accordingly."
        ),
    };
    request.push_turn(ConversationTurn::new(ConversationRole::System, guidance));
}

fn goal_keyword(request: &AgentRequest) -> String {
    request
        .intent
        .primary_goal
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| request.goal.as_str())
        .trim()
        .to_string()
}

fn ensure_target_site_hint(request: &mut AgentRequest, site: &str) {
    let already_present = request
        .intent
        .target_sites
        .iter()
        .any(|existing| existing.contains(site));
    if !already_present {
        request.intent.target_sites.push(site.to_string());
        request.intent.target_sites_are_hints = true;
        let values: Vec<Value> = request
            .intent
            .target_sites
            .iter()
            .map(|value| Value::String(value.clone()))
            .collect();
        request
            .metadata
            .insert("target_sites".to_string(), Value::Array(values));
    }
}

fn set_search_terms(request: &mut AgentRequest, hints: Vec<String>) {
    let mut combined = Vec::new();
    if let Some(Value::Array(existing)) = request.metadata.get("search_terms") {
        for entry in existing {
            if let Some(term) = entry.as_str() {
                let trimmed = term.trim();
                if !trimmed.is_empty() {
                    combined.push(trimmed.to_string());
                }
            }
        }
    }
    for hint in hints {
        let trimmed = hint.trim();
        if trimmed.is_empty() {
            continue;
        }
        if combined
            .iter()
            .any(|existing| existing.eq_ignore_ascii_case(trimmed))
        {
            continue;
        }
        combined.push(trimmed.to_string());
    }
    if combined.is_empty() {
        request.metadata.remove("search_terms");
    } else {
        let values: Vec<Value> = combined
            .into_iter()
            .map(|term| Value::String(term))
            .collect();
        request
            .metadata
            .insert("search_terms".to_string(), Value::Array(values));
    }
}

fn search_hint(goal: &str) -> String {
    let encoded: String = form_urlencoded::byte_serialize(goal.as_bytes()).collect();
    format!("https://www.baidu.com/s?wd={encoded}%20行情")
}

fn latest_evaluation_summary(report: &FlowExecutionReport) -> Option<String> {
    report
        .steps
        .iter()
        .rev()
        .find(|step| step.tool_kind.eq_ignore_ascii_case("agent.evaluate"))
        .and_then(|step| step.observation_summary.clone())
}

fn recent_step_history(report: &FlowExecutionReport, limit: usize) -> Option<String> {
    if limit == 0 || report.steps.is_empty() {
        return None;
    }
    let start = report.steps.len().saturating_sub(limit);
    let mut lines = Vec::new();
    for (index, step) in report.steps.iter().enumerate().skip(start) {
        let status_icon = match step.status {
            StepExecutionStatus::Success => "✅",
            StepExecutionStatus::Failed => "❌",
        };
        let summary = step
            .observation_summary
            .as_deref()
            .or(step.error.as_deref())
            .unwrap_or("未提供额外说明");
        lines.push(format!(
            "{status_icon} Step {idx}: {title} [{tool}] — {summary}",
            idx = index + 1,
            title = step.title,
            tool = step.tool_kind,
            summary = truncate_history_summary(summary, HISTORY_SUMMARY_LIMIT),
        ));
    }
    if lines.is_empty() {
        None
    } else {
        Some(format!("Recent execution trace:\n{}", lines.join("\n")))
    }
}

fn truncate_history_summary(value: &str, limit: usize) -> String {
    if value.chars().count() <= limit {
        return value.to_string();
    }
    let mut shortened = String::new();
    for (index, ch) in value.chars().enumerate() {
        if index >= limit {
            shortened.push('…');
            break;
        }
        shortened.push(ch);
    }
    shortened
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_core::{AgentRequest, ConversationRole};
    use soulbrowser_core_types::TaskId;

    use crate::agent::executor::{FlowExecutionReport, StepExecutionReport, StepExecutionStatus};

    fn sample_report(
        summary: Option<&str>,
        blocker: Option<&str>,
        evaluation: Option<&str>,
    ) -> FlowExecutionReport {
        FlowExecutionReport {
            success: false,
            steps: vec![
                StepExecutionReport {
                    step_id: "step-0".to_string(),
                    title: "评估页面".to_string(),
                    tool_kind: "agent.evaluate".to_string(),
                    status: StepExecutionStatus::Success,
                    attempts: 1,
                    error: None,
                    dispatches: Vec::new(),
                    total_wait_ms: 0,
                    total_run_ms: 0,
                    observation_summary: evaluation.map(|s| s.to_string()),
                    blocker_kind: None,
                    agent_state: None,
                },
                StepExecutionReport {
                    step_id: "step-1".to_string(),
                    title: "采集行情".to_string(),
                    tool_kind: "data.extract-site".to_string(),
                    status: StepExecutionStatus::Failed,
                    attempts: 1,
                    error: Some("Observation blocked".to_string()),
                    dispatches: Vec::new(),
                    total_wait_ms: 0,
                    total_run_ms: 0,
                    observation_summary: summary.map(|s| s.to_string()),
                    blocker_kind: blocker.map(|s| s.to_string()),
                    agent_state: None,
                },
            ],
            user_results: Vec::new(),
            missing_user_result: true,
            memory_log: Vec::new(),
            judge_verdict: None,
        }
    }

    #[test]
    fn injects_summary_and_blocker_guidance() {
        let request = AgentRequest::new(TaskId::new(), "查询白银行情");
        let report = sample_report(
            Some("《白银》: 页面不存在"),
            Some("page_not_found"),
            Some("页面仍为404，跳出推广页"),
        );
        let (next_request, failure_summary) = augment_request_for_replan(
            &request,
            &report,
            0,
            Some("《白银》: 页面不存在"),
            Some("page_not_found"),
            None,
        )
        .expect("replan context");

        assert!(failure_summary.contains("采集行情"));
        assert!(failure_summary.contains("Latest evaluation"));
        let blocker_meta = next_request
            .metadata
            .get("registry_blocker_kind")
            .and_then(Value::as_str)
            .unwrap();
        assert_eq!(blocker_meta, "page_not_found");
        let evaluation_meta = next_request
            .metadata
            .get("latest_evaluation_summary")
            .and_then(Value::as_str)
            .unwrap();
        assert_eq!(evaluation_meta, "页面仍为404，跳出推广页");
        let has_baidu_hint = next_request
            .conversation
            .iter()
            .filter(|turn| matches!(turn.role, ConversationRole::System))
            .any(|turn| turn.message.contains("baidu.com"));
        assert!(has_baidu_hint);
        let eastmoney_hint = next_request
            .conversation
            .iter()
            .filter(|turn| matches!(turn.role, ConversationRole::System))
            .any(|turn| turn.message.contains("东方财富"));
        assert!(eastmoney_hint);
        let search_terms = next_request
            .metadata
            .get("search_terms")
            .and_then(Value::as_array)
            .expect("search terms");
        assert!(search_terms
            .iter()
            .any(|term| term.as_str().unwrap_or("").contains("东方财富")));
        let site_hints = next_request
            .metadata
            .get("target_sites")
            .and_then(Value::as_array)
            .expect("target sites");
        assert!(site_hints
            .iter()
            .any(|value| value.as_str().unwrap_or("").contains("quote.eastmoney.com")));
        let history_prompt = next_request
            .metadata
            .get("agent_history_prompt")
            .and_then(Value::as_str)
            .unwrap();
        assert!(history_prompt.contains("Step 1"));
    }

    #[test]
    fn injects_search_blocker_guidance() {
        let request = AgentRequest::new(TaskId::new(), "查找黄金走势");
        let report = sample_report(None, Some("search_no_results"), None);
        let (next_request, _) =
            augment_request_for_replan(&request, &report, 0, None, Some("search_no_results"), None)
                .expect("replan context");

        let guidance = next_request
            .conversation
            .iter()
            .filter(|turn| matches!(turn.role, ConversationRole::System))
            .find(|turn| turn.message.contains("搜索结果区域"))
            .map(|turn| turn.message.clone())
            .expect("search guidance");
        assert!(guidance.contains("调整搜索词"));
        let search_terms = next_request
            .metadata
            .get("search_terms")
            .and_then(Value::as_array)
            .expect("search metadata");
        assert!(search_terms
            .iter()
            .any(|term| term.as_str().unwrap_or("").contains("东方财富")));
    }

    #[test]
    fn injects_popup_blocker_guidance() {
        let request = AgentRequest::new(TaskId::new(), "处理弹窗");
        let report = sample_report(None, Some("popup_unclosed"), None);
        let (next_request, _) =
            augment_request_for_replan(&request, &report, 0, None, Some("popup_unclosed"), None)
                .expect("replan context");

        let guidance = next_request
            .conversation
            .iter()
            .filter(|turn| matches!(turn.role, ConversationRole::System))
            .find(|turn| turn.message.contains("弹窗"))
            .map(|turn| turn.message.clone())
            .expect("popup guidance");
        assert!(guidance.contains("browser.close-modal"));
    }

    #[test]
    fn injects_quote_fetch_hints() {
        let request = AgentRequest::new(TaskId::new(), "查询黄金");
        let report = sample_report(None, Some("quote_fetch_failed"), None);
        let (next_request, _) = augment_request_for_replan(
            &request,
            &report,
            0,
            None,
            Some("quote_fetch_failed"),
            None,
        )
        .expect("replan context");
        let search_terms = next_request
            .metadata
            .get("search_terms")
            .and_then(Value::as_array)
            .expect("search metadata");
        assert!(search_terms
            .iter()
            .any(|term| term.as_str().unwrap_or("").contains("新浪财经")));
        let site_hints = next_request
            .metadata
            .get("target_sites")
            .and_then(Value::as_array)
            .expect("target sites");
        assert!(site_hints
            .iter()
            .any(|value| value.as_str().unwrap_or("").contains("quote.eastmoney.com")));
    }
}
