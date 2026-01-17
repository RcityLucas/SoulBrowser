use agent_core::{
    AgentIntentKind, AgentIntentMetadata, AgentPlanner, AgentRequest, AgentToolKind, PlanValidator,
    RequestedOutput, RuleBasedPlanner,
};
use soulbrowser_core_types::TaskId;

#[test]
fn search_market_info_intent_uses_recipe() {
    let mut request = AgentRequest::new(TaskId::new(), "搜行情");
    request.intent = AgentIntentMetadata {
        intent_id: Some("search_market_info".to_string()),
        primary_goal: Some("搜行情".to_string()),
        target_sites: vec!["https://www.baidu.com".to_string()],
        target_sites_are_hints: false,
        required_outputs: vec![RequestedOutput::new("market_info_v1.json")],
        preferred_language: Some("zh-CN".to_string()),
        blocker_remediations: Default::default(),
        validation_keywords: Vec::new(),
        allowed_domains: Vec::new(),
        intent_kind: AgentIntentKind::Operational,
    };

    let planner = RuleBasedPlanner::default();
    let outcome = planner.draft_plan(&request).expect("plan success");
    let plan = outcome.plan;

    let first_step = plan.steps.first().expect("plan has steps");
    if let AgentToolKind::Navigate { url, .. } = &first_step.tool.kind {
        assert!(url.contains("baidu"));
    } else {
        panic!("first step should navigate to preferred site");
    }

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
        target_sites_are_hints: false,
        required_outputs: vec![RequestedOutput::new("news_brief_v1.json")],
        preferred_language: Some("zh-CN".to_string()),
        blocker_remediations: Default::default(),
        validation_keywords: Vec::new(),
        allowed_domains: Vec::new(),
        intent_kind: AgentIntentKind::Operational,
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

#[test]
fn market_quote_lookup_intent_uses_recipe() {
    let mut request = AgentRequest::new(TaskId::new(), "查询铜的价格");
    request.intent = AgentIntentMetadata {
        intent_id: Some("market_quote_lookup".to_string()),
        primary_goal: Some("查询铜的价格".to_string()),
        target_sites: vec![],
        target_sites_are_hints: false,
        required_outputs: vec![RequestedOutput::new("metal_price_v1.json")],
        preferred_language: Some("zh-CN".to_string()),
        blocker_remediations: Default::default(),
        validation_keywords: Vec::new(),
        allowed_domains: Vec::new(),
        intent_kind: AgentIntentKind::Operational,
    };

    let planner = RuleBasedPlanner::default();
    let outcome = planner.draft_plan(&request).expect("plan success");
    let plan = outcome.plan;

    assert!(plan
        .steps
        .first()
        .map(|step| {
            matches!(
                step.tool.kind,
                AgentToolKind::Custom { ref name, .. } if name == "browser.search"
            )
        })
        .unwrap_or(false));

    assert!(plan
        .steps
        .get(1)
        .map(|step| matches!(step.tool.kind, AgentToolKind::Navigate { .. }))
        .unwrap_or(false));

    assert!(plan.steps.iter().any(|step| {
        matches!(
            &step.tool.kind,
            AgentToolKind::Custom { name, .. } if name == "market.quote.fetch"
        )
    }));

    let parse_step = plan.steps.iter().find(|step| {
        matches!(
            &step.tool.kind,
            AgentToolKind::Custom { name, .. } if name == "data.parse.metal_price"
        )
    });
    let parse_step = parse_step.expect("parse step present");
    if let AgentToolKind::Custom { payload, .. } = &parse_step.tool.kind {
        assert!(payload.get("source_step_id").is_some());
    }

    assert!(plan.steps.iter().any(|step| {
        matches!(
            &step.tool.kind,
            AgentToolKind::Custom { name, .. } if name == "data.validate.metal_price"
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
            Some("metal_price_v1.json")
        );
        assert_eq!(
            payload.get("artifact_label").and_then(|v| v.as_str()),
            Some("structured.metal_price_v1")
        );
    }
}

#[test]
fn fetch_weather_intent_uses_recipe() {
    let mut request = AgentRequest::new(TaskId::new(), "查询深圳天气");
    request.intent = AgentIntentMetadata {
        intent_id: Some("fetch_weather".to_string()),
        primary_goal: Some("查询深圳天气".to_string()),
        target_sites: vec!["https://www.baidu.com".to_string()],
        target_sites_are_hints: false,
        required_outputs: vec![RequestedOutput::new("weather_report_v1.json")],
        preferred_language: Some("zh-CN".to_string()),
        blocker_remediations: Default::default(),
        validation_keywords: Vec::new(),
        allowed_domains: Vec::new(),
        intent_kind: AgentIntentKind::Informational,
    };

    let planner = RuleBasedPlanner::default();
    let outcome = planner.draft_plan(&request).expect("weather plan");
    let plan = outcome.plan;

    assert_eq!(plan.steps.len(), 6, "weather template should emit 6 steps");
    assert!(matches!(
        plan.steps[0].tool.kind,
        AgentToolKind::Navigate { .. }
    ));
    assert!(matches!(
        plan.steps[1].tool.kind,
        AgentToolKind::TypeText { .. }
    ));
    assert!(plan.steps.iter().any(|step| match &step.tool.kind {
        AgentToolKind::Custom { name, .. } => name == "data.parse.weather",
        _ => false,
    }));
    assert!(plan.steps.iter().any(|step| match &step.tool.kind {
        AgentToolKind::Custom { name, payload } if name == "data.deliver.structured" => {
            payload
                .get("schema")
                .and_then(|value| value.as_str())
                .map(|schema| schema.eq_ignore_ascii_case("weather_report_v1"))
                .unwrap_or(false)
        }
        _ => false,
    }));

    PlanValidator::strict()
        .validate(&plan, &request)
        .expect("weather plan passes validation");
}
