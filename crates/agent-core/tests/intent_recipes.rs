use agent_core::{
    AgentIntentMetadata, AgentPlanner, AgentRequest, AgentToolKind, RequestedOutput,
    RuleBasedPlanner,
};
use soulbrowser_core_types::TaskId;

#[test]
fn search_market_info_intent_uses_recipe() {
    let mut request = AgentRequest::new(TaskId::new(), "搜行情");
    request.intent = AgentIntentMetadata {
        intent_id: Some("search_market_info".to_string()),
        primary_goal: Some("搜行情".to_string()),
        target_sites: vec!["https://www.baidu.com".to_string()],
        required_outputs: vec![RequestedOutput::new("market_info_v1.json")],
        preferred_language: Some("zh-CN".to_string()),
        blocker_remediations: Default::default(),
    };

    let planner = RuleBasedPlanner::default();
    let outcome = planner.draft_plan(&request).expect("plan success");
    let plan = outcome.plan;

    assert!(plan
        .steps
        .first()
        .map(|step| matches!(step.tool.kind, AgentToolKind::Navigate { .. }))
        .unwrap_or(false));

    assert!(plan.steps.iter().any(|step| {
        matches!(
            &step.tool.kind,
            AgentToolKind::Custom { name, .. } if name == "data.parse.market_info"
        )
    }));

    let deliver = plan.steps.iter().find(|step| {
        matches!(
            &step.tool.kind,
            AgentToolKind::Custom { name, .. } if name == "data.deliver.structured"
        )
    });
    let deliver = deliver.expect("deliver step present");
    if let AgentToolKind::Custom { payload, .. } = &deliver.tool.kind {
        assert_eq!(
            payload.get("schema").and_then(|v| v.as_str()),
            Some("market_info_v1.json")
        );
    }
}

#[test]
fn summarize_news_intent_uses_recipe() {
    let mut request = AgentRequest::new(TaskId::new(), "新闻摘要");
    request.intent = AgentIntentMetadata {
        intent_id: Some("summarize_news".to_string()),
        primary_goal: Some("新闻摘要".to_string()),
        target_sites: vec!["https://news.google.com".to_string()],
        required_outputs: vec![RequestedOutput::new("news_brief_v1.json")],
        preferred_language: Some("zh-CN".to_string()),
        blocker_remediations: Default::default(),
    };

    let planner = RuleBasedPlanner::default();
    let outcome = planner.draft_plan(&request).expect("plan success");
    let plan = outcome.plan;

    assert!(plan
        .steps
        .iter()
        .any(|step| matches!(step.tool.kind, AgentToolKind::Navigate { .. })));
    assert!(plan.steps.iter().any(|step| {
        matches!(
            &step.tool.kind,
            AgentToolKind::Custom { name, .. } if name == "data.parse.news_brief"
        )
    }));
    let deliver = plan.steps.iter().find(|step| {
        matches!(
            &step.tool.kind,
            AgentToolKind::Custom { name, .. } if name == "data.deliver.structured"
        )
    });
    let deliver = deliver.expect("deliver step present");
    if let AgentToolKind::Custom { payload, .. } = &deliver.tool.kind {
        assert_eq!(
            payload.get("schema").and_then(|v| v.as_str()),
            Some("news_brief_v1.json")
        );
    }
}
