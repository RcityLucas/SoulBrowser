use crate::errors::AgentError;
use crate::guardrails::{derive_guardrail_domains, derive_guardrail_keywords};
use crate::model::AgentRequest;
use crate::plan::{
    AgentLocator, AgentPlan, AgentPlanMeta, AgentPlanStep, AgentScrollTarget, AgentTool,
    AgentToolKind, AgentValidation, AgentWaitCondition, WaitMode,
};
use crate::planner::{
    resolve_quote_plan, AgentPlanner, PlanStageGraph, PlannerConfig, PlannerOutcome, QuoteQuery,
};
use crate::weather::{first_weather_subject, weather_query_text, weather_search_url};
use chrono::{Datelike, Utc};
use once_cell::sync::Lazy;
use regex::Regex;
use serde_json::{json, Number as JsonNumber, Value};
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use url::{form_urlencoded, Url};

static STAGE_GRAPH: Lazy<PlanStageGraph> =
    Lazy::new(|| PlanStageGraph::load_from_env_or_default().unwrap_or_default());

/// Simple rule-based planner that turns user goals into structured steps.
#[derive(Debug, Clone)]
pub struct RuleBasedPlanner {
    config: PlannerConfig,
}

impl RuleBasedPlanner {
    pub fn new(config: PlannerConfig) -> Self {
        Self { config }
    }

    fn default_title(goal: &str) -> String {
        let trimmed = goal.trim();
        if trimmed.is_empty() {
            "Agent task".to_string()
        } else if trimmed.len() <= 72 {
            trimmed.to_string()
        } else {
            format!("{}…", trimmed.chars().take(69).collect::<String>())
        }
    }

    fn apply_intent_recipe(&self, request: &AgentRequest) -> Option<PlannerOutcome> {
        let intent_id = request.intent.intent_id.as_deref()?;
        match intent_id {
            "search_market_info" => Some(self.build_market_info_recipe(request)),
            "market_quote_lookup" => Some(self.build_market_quote_recipe(request)),
            "summarize_news" => Some(self.build_news_recipe(request)),
            "fetch_weather" => Some(self.build_weather_recipe(request)),
            _ => None,
        }
    }

    fn build_market_quote_recipe(&self, request: &AgentRequest) -> PlannerOutcome {
        let mut plan = AgentPlan::new(request.task_id.clone(), Self::default_title(&request.goal));
        let quote_context = QuoteIntentContext::from_request(request);
        let guardrail_domains = derive_guardrail_domains(request);
        let guardrail_keywords = derive_guardrail_keywords(request);
        let plan_spec = resolve_quote_plan(request, quote_context.as_query());
        let search_step_id = step_id(plan.steps.len());
        let search_query = format!(
            "{} {} 行情",
            quote_context.metal_label, quote_context.contract
        );
        let site_hint = Url::parse(&plan_spec.navigate_url)
            .ok()
            .and_then(|url| url.host_str().map(|host| host.to_string()));
        if !guardrail_keywords.is_empty() {
            plan.meta.vendor_context.insert(
                "guardrail_keywords".to_string(),
                json!({
                    "keywords": guardrail_keywords,
                    "count": guardrail_keywords.len(),
                    "emitted": false,
                }),
            );
        }
        let mut search_payload = json!({
            "query": guardrail_search_query(&search_query, &guardrail_domains),
        });
        plan.push_step(
            AgentPlanStep::new(
                search_step_id,
                format!("搜索{}行情入口", quote_context.metal_label),
                AgentTool {
                    kind: AgentToolKind::Custom {
                        name: "browser.search".to_string(),
                        payload: search_payload,
                    },
                    wait: WaitMode::DomReady,
                    timeout_ms: Some(30_000),
                },
            )
            .with_detail(format!(
                "使用搜索引擎定位可靠的{}报价页面，必要时可更换站点",
                quote_context.metal_label
            )),
        );
        let target_domain = site_hint.clone();
        let auto_act_domains = auto_act_domain_hints(target_domain.clone(), &guardrail_domains);
        let refresh_queries = guardrail_refresh_queries(&search_query, &auto_act_domains);
        if !auto_act_domains.is_empty() {
            plan.meta.vendor_context.insert(
                "auto_act_engine".to_string(),
                json!({
                    "engine": "baidu",
                    "domains": auto_act_domains.clone(),
                    "emitted": false,
                }),
            );
        }
        let auto_click_id = step_id(plan.steps.len());
        let auto_detail = if auto_act_domains.is_empty() {
            format!("自动点击{}的搜索结果", quote_context.metal_label)
        } else {
            format!("优先打开 {} 的搜索结果", auto_act_domains.join(" / "))
        };
        let auto_payload = json!({
            "engine": "baidu",
            "selectors": ["div#content_left h3 a", "div#content_left a"],
            "domains": auto_act_domains,
            "max_candidates": 40,
            "max_attempts": 3,
            "wait_per_candidate_ms": 15_000,
            "guardrail_queries": refresh_queries,
        });
        let mut auto_step = AgentPlanStep::new(
            auto_click_id.clone(),
            format!("打开{}权威结果", quote_context.metal_label),
            AgentTool {
                kind: AgentToolKind::Custom {
                    name: "browser.search.click-result".to_string(),
                    payload: auto_payload,
                },
                wait: WaitMode::DomReady,
                timeout_ms: Some(60_000),
            },
        )
        .with_detail(auto_detail);
        if let Some(ref domain) = target_domain {
            auto_step
                .metadata
                .insert("expected_url".to_string(), json!(domain));
        }
        if !refresh_queries.is_empty() {
            let max_retries = refresh_queries.len().min(3) as u32;
            auto_step.metadata.insert(
                "auto_act_refresh".to_string(),
                json!({
                    "engine": "baidu",
                    "queries": refresh_queries,
                    "max_retries": max_retries,
                }),
            );
        }
        plan.push_step(auto_step);
        let observe_url = plan_spec.navigate_url.clone();
        let mut navigate_step = AgentPlanStep::new(
            step_id(plan.steps.len()),
            "选择行情数据源并打开",
            AgentTool {
                kind: AgentToolKind::Navigate {
                    url: plan_spec.navigate_url.clone(),
                },
                wait: WaitMode::DomReady,
                timeout_ms: Some(30_000),
            },
        )
        .with_detail(format!(
            "打开{}行情页面（来源：{}）",
            quote_context.metal_label, observe_url
        ));
        if let Some(ref domain) = target_domain {
            navigate_step
                .metadata
                .insert("expected_url".to_string(), json!(domain));
        }
        plan.push_step(navigate_step);

        let observe_step_id = step_id(plan.steps.len());
        plan.push_step(
            AgentPlanStep::new(
                observe_step_id.clone(),
                "采集行情快照",
                AgentTool {
                    kind: AgentToolKind::Custom {
                        name: "market.quote.fetch".to_string(),
                        payload: plan_spec.payload.clone(),
                    },
                    wait: WaitMode::None,
                    timeout_ms: Some(10_000),
                },
            )
            .with_detail("调用行情 API 并在页面中抓取备选表格/要素"),
        );

        let validation_keywords =
            assemble_validation_keywords(request, &quote_context.validation_keywords());
        let candidate_urls =
            candidate_urls_from_payload(&plan_spec.payload, &plan_spec.navigate_url);
        let allowed_domains =
            collect_allowed_domains(&request.intent.allowed_domains, &candidate_urls);
        if let Some(step) = build_target_validation_step(
            plan.steps.len(),
            &observe_step_id,
            &validation_keywords,
            &allowed_domains,
            Some(200),
            &format!("确认{}行情页面有效", quote_context.metal_label),
        ) {
            plan.push_step(step);
        }

        let parse_step_id = step_id(plan.steps.len());
        plan.push_step(AgentPlanStep::new(
            parse_step_id.clone(),
            "解析金属价格",
            AgentTool {
                kind: AgentToolKind::Custom {
                    name: "data.parse.metal_price".to_string(),
                    payload: json!({
                        "source_step_id": observe_step_id,
                        "detail": "结构化提取金属行情表格",
                    }),
                },
                wait: WaitMode::None,
                timeout_ms: Some(5_000),
            },
        ));

        let validate_step_id = step_id(plan.steps.len());
        plan.push_step(AgentPlanStep::new(
            validate_step_id.clone(),
            "校验金属行情有效性",
            AgentTool {
                kind: AgentToolKind::Custom {
                    name: "data.validate.metal_price".to_string(),
                    payload: json!({
                        "source_step_id": parse_step_id,
                        "metal_keyword": quote_context.metal_label,
                        "allowed_markets": quote_context.allowed_markets,
                        "max_age_hours": 24,
                    }),
                },
                wait: WaitMode::None,
                timeout_ms: Some(3_000),
            },
        ));

        let schema = required_schema(request, "metal_price_v1.json");
        plan.push_step(AgentPlanStep::new(
            step_id(plan.steps.len()),
            "交付金属报价结果",
            AgentTool {
                kind: AgentToolKind::Custom {
                    name: "data.deliver.structured".to_string(),
                    payload: json!({
                        "schema": schema,
                        "artifact_label": "structured.metal_price_v1",
                        "filename": schema,
                        "source_step_id": parse_step_id,
                        "target_url": observe_url,
                    }),
                },
                wait: WaitMode::None,
                timeout_ms: Some(2_000),
            },
        ));

        plan.meta = recipe_meta(request, "market_quote_lookup");
        attach_stage_metadata(&mut plan, request);
        PlannerOutcome {
            plan,
            explanations: vec!["Intent recipe market_quote_lookup applied".to_string()],
        }
    }

    fn build_market_info_recipe(&self, request: &AgentRequest) -> PlannerOutcome {
        let mut plan = AgentPlan::new(request.task_id.clone(), Self::default_title(&request.goal));
        let url = preferred_intent_site(request, "https://www.baidu.com");
        plan.push_step(AgentPlanStep::new(
            step_id(plan.steps.len()),
            "Navigate to market source",
            AgentTool {
                kind: AgentToolKind::Navigate { url: url.clone() },
                wait: WaitMode::DomReady,
                timeout_ms: Some(30_000),
            },
        ));

        let parse_step_id = step_id(plan.steps.len());
        plan.push_step(AgentPlanStep::new(
            parse_step_id.clone(),
            "Parse market data",
            AgentTool {
                kind: AgentToolKind::Custom {
                    name: "data.parse.market_info".to_string(),
                    payload: json!({}),
                },
                wait: WaitMode::None,
                timeout_ms: Some(5_000),
            },
        ));

        let schema = required_schema(request, "market_info_v1.json");
        plan.push_step(AgentPlanStep::new(
            step_id(plan.steps.len()),
            "Deliver market snapshot",
            AgentTool {
                kind: AgentToolKind::Custom {
                    name: "data.deliver.structured".to_string(),
                    payload: json!({
                        "schema": schema,
                        "artifact_label": "market_info",
                        "filename": schema,
                        "source_step_id": parse_step_id,
                        "target_url": url,
                    }),
                },
                wait: WaitMode::None,
                timeout_ms: Some(2_000),
            },
        ));

        plan.meta = recipe_meta(request, "search_market_info");
        attach_stage_metadata(&mut plan, request);
        PlannerOutcome {
            plan,
            explanations: vec!["Intent recipe search_market_info applied".to_string()],
        }
    }

    fn build_news_recipe(&self, request: &AgentRequest) -> PlannerOutcome {
        let mut plan = AgentPlan::new(request.task_id.clone(), Self::default_title(&request.goal));
        let url = preferred_intent_site(request, "https://news.google.com");
        plan.push_step(AgentPlanStep::new(
            step_id(plan.steps.len()),
            "Navigate to news source",
            AgentTool {
                kind: AgentToolKind::Navigate { url: url.clone() },
                wait: WaitMode::DomReady,
                timeout_ms: Some(30_000),
            },
        ));

        let parse_step_id = step_id(plan.steps.len());
        plan.push_step(AgentPlanStep::new(
            parse_step_id.clone(),
            "Parse news brief",
            AgentTool {
                kind: AgentToolKind::Custom {
                    name: "data.parse.news_brief".to_string(),
                    payload: json!({}),
                },
                wait: WaitMode::None,
                timeout_ms: Some(5_000),
            },
        ));

        let schema = required_schema(request, "news_brief_v1.json");
        plan.push_step(AgentPlanStep::new(
            step_id(plan.steps.len()),
            "Deliver news brief",
            AgentTool {
                kind: AgentToolKind::Custom {
                    name: "data.deliver.structured".to_string(),
                    payload: json!({
                        "schema": schema,
                        "artifact_label": "news_brief",
                        "filename": schema,
                        "source_step_id": parse_step_id,
                        "target_url": url,
                    }),
                },
                wait: WaitMode::None,
                timeout_ms: Some(2_000),
            },
        ));

        plan.meta = recipe_meta(request, "summarize_news");
        attach_stage_metadata(&mut plan, request);
        PlannerOutcome {
            plan,
            explanations: vec!["Intent recipe summarize_news applied".to_string()],
        }
    }

    fn build_weather_recipe(&self, request: &AgentRequest) -> PlannerOutcome {
        let mut plan = AgentPlan::new(request.task_id.clone(), Self::default_title(&request.goal));
        let start_url = preferred_intent_site(request, "https://www.baidu.com");

        let mut navigate = AgentPlanStep::new(
            step_id(plan.steps.len()),
            "Navigate to Baidu",
            AgentTool {
                kind: AgentToolKind::Navigate {
                    url: start_url.clone(),
                },
                wait: WaitMode::DomReady,
                timeout_ms: Some(30_000),
            },
        )
        .with_detail(format!("Open weather portal at {start_url}"));
        navigate.validations.push(AgentValidation {
            description: "Ensure Baidu home is visible".to_string(),
            condition: AgentWaitCondition::UrlMatches(start_url.clone()),
        });
        plan.push_step(navigate);

        let query_text = weather_query_text(request);
        let search_locator = AgentLocator::Css("input#kw".to_string());
        let mut type_step = AgentPlanStep::new(
            step_id(plan.steps.len()),
            "Search weather",
            AgentTool {
                kind: AgentToolKind::TypeText {
                    locator: search_locator.clone(),
                    text: query_text.clone(),
                    submit: true,
                },
                wait: WaitMode::DomReady,
                timeout_ms: Some(10_000),
            },
        )
        .with_detail(format!("Search Baidu for {query_text}"));
        type_step.validations.push(AgentValidation {
            description: "Search box ready".to_string(),
            condition: AgentWaitCondition::ElementVisible(search_locator.clone()),
        });
        plan.push_step(type_step);

        let weather_locator = AgentLocator::Text {
            content: "天气".to_string(),
            exact: false,
        };
        let mut wait_step = AgentPlanStep::new(
            step_id(plan.steps.len()),
            "Wait for weather results",
            AgentTool {
                kind: AgentToolKind::Wait {
                    condition: AgentWaitCondition::ElementVisible(weather_locator.clone()),
                },
                wait: WaitMode::None,
                timeout_ms: Some(20_000),
            },
        )
        .with_detail("Ensure the weather widget is visible on the results page");
        wait_step.validations.push(AgentValidation {
            description: "Weather widget became visible".to_string(),
            condition: AgentWaitCondition::ElementVisible(weather_locator),
        });
        plan.push_step(wait_step);

        let observe_id = step_id(plan.steps.len());
        let search_url = weather_search_url(request);
        plan.push_step(
            AgentPlanStep::new(
                observe_id.clone(),
                "Collect weather snapshot",
                AgentTool {
                    kind: AgentToolKind::Custom {
                        name: "data.extract-site".to_string(),
                        payload: json!({
                            "url": search_url,
                            "title": "百度天气搜索结果",
                            "detail": format!("Capture weather results for {query_text}"),
                        }),
                    },
                    wait: WaitMode::None,
                    timeout_ms: Some(10_000),
                },
            )
            .with_detail("Capture Baidu weather results for downstream parsing"),
        );

        let parse_step_id = step_id(plan.steps.len());
        plan.push_step(
            AgentPlanStep::new(
                parse_step_id.clone(),
                "解析天气数据",
                AgentTool {
                    kind: AgentToolKind::Custom {
                        name: "data.parse.weather".to_string(),
                        payload: json!({
                            "source_step_id": observe_id,
                            "title": "Parse Baidu weather",
                            "detail": "Extract weather_report_v1 from Baidu",
                        }),
                    },
                    wait: WaitMode::Idle,
                    timeout_ms: Some(8_000),
                },
            )
            .with_detail("Parse Baidu weather widget into weather_report_v1"),
        );

        plan.push_step(
            AgentPlanStep::new(
                step_id(plan.steps.len()),
                "交付天气结构化结果",
                AgentTool {
                    kind: AgentToolKind::Custom {
                        name: "data.deliver.structured".to_string(),
                        payload: json!({
                            "schema": "weather_report_v1",
                            "artifact_label": "structured.weather_report_v1",
                            "filename": "weather_report_v1.json",
                            "source_step_id": parse_step_id,
                        }),
                    },
                    wait: WaitMode::None,
                    timeout_ms: Some(4_000),
                },
            )
            .with_detail("Return structured weather report"),
        );

        plan.meta = recipe_meta(request, "fetch_weather");
        attach_stage_metadata(&mut plan, request);
        PlannerOutcome {
            plan,
            explanations: vec!["Intent recipe fetch_weather applied".to_string()],
        }
    }

    fn build_scaffold_plan(&self, request: &AgentRequest) -> PlannerOutcome {
        let mut plan = AgentPlan::new(request.task_id.clone(), Self::default_title(&request.goal))
            .with_description(format!("Auto scaffolded for goal: {}", request.goal.trim()));

        let default_url = canonical_url_for_request(request);
        let mut steps: Vec<AgentPlanStep> = Vec::new();

        let nav_id = step_id(steps.len());
        let mut nav_step = AgentPlanStep::new(
            nav_id.clone(),
            "Navigate to reference site",
            AgentTool {
                kind: AgentToolKind::Navigate {
                    url: default_url.clone(),
                },
                wait: WaitMode::DomReady,
                timeout_ms: Some(30_000),
            },
        )
        .with_detail(format!("Open {default_url}"));
        nav_step.validations.push(AgentValidation {
            description: "Ensure navigation reached target".to_string(),
            condition: AgentWaitCondition::UrlMatches(default_url.clone()),
        });
        steps.push(nav_step);

        let observe_id = step_id(steps.len());
        steps.push(
            AgentPlanStep::new(
                observe_id.clone(),
                "Collect page snapshot",
                AgentTool {
                    kind: AgentToolKind::Custom {
                        name: "data.extract-site".to_string(),
                        payload: json!({
                            "url": default_url,
                            "title": "Auto observation",
                            "detail": "Synthesized observation for downstream parsing",
                        }),
                    },
                    wait: WaitMode::None,
                    timeout_ms: Some(10_000),
                },
            )
            .with_detail("Capture the destination page for parsing"),
        );

        let validation_keywords = assemble_validation_keywords(request, &[]);
        let allowed_domains =
            collect_allowed_domains(&request.intent.allowed_domains, &[default_url.clone()]);
        if let Some(step) = build_target_validation_step(
            steps.len(),
            &observe_id,
            &validation_keywords,
            &allowed_domains,
            Some(200),
            "确保采集页面与目标一致",
        ) {
            steps.push(step);
        }

        if let Some(schema) = first_required_schema(request) {
            let parse_id = step_id(steps.len());
            steps.push(
                AgentPlanStep::new(
                    parse_id.clone(),
                    "Parse structured data",
                    AgentTool {
                        kind: AgentToolKind::Custom {
                            name: "data.parse.generic".to_string(),
                            payload: json!({
                                "schema": schema,
                                "source_step_id": observe_id,
                                "title": "Auto parser",
                                "detail": "Synthesized parser for structured output",
                            }),
                        },
                        wait: WaitMode::None,
                        timeout_ms: Some(5_000),
                    },
                )
                .with_detail("Convert observation into structured schema"),
            );

            let deliver_filename = format!("{}.json", schema);
            steps.push(
                AgentPlanStep::new(
                    step_id(steps.len()),
                    "Deliver structured result",
                    AgentTool {
                        kind: AgentToolKind::Custom {
                            name: "data.deliver.structured".to_string(),
                            payload: json!({
                                "schema": schema,
                                "artifact_label": format!("structured.{}", schema),
                                "filename": deliver_filename,
                                "source_step_id": parse_id,
                            }),
                        },
                        wait: WaitMode::None,
                        timeout_ms: Some(2_000),
                    },
                )
                .with_detail("Return machine-readable result"),
            );
        } else {
            steps.push(
                AgentPlanStep::new(
                    step_id(steps.len()),
                    "Summarize findings",
                    AgentTool {
                        kind: AgentToolKind::Custom {
                            name: "agent.note".to_string(),
                            payload: json!({
                                "title": "Auto summary",
                                "detail": request.goal.trim(),
                            }),
                        },
                        wait: WaitMode::None,
                        timeout_ms: Some(2_000),
                    },
                )
                .with_detail("Report findings back to user"),
            );
        }

        for step in steps.into_iter() {
            plan.push_step(step);
        }

        plan.meta = AgentPlanMeta {
            rationale: vec!["Synthesized scaffold plan".to_string()],
            risk_assessment: vec!["Auto scaffold".to_string()],
            vendor_context: vendor_context_snapshot(request),
            overlays: Vec::new(),
        };

        attach_stage_metadata(&mut plan, request);
        PlannerOutcome {
            plan,
            explanations: vec!["Synthesized fallback navigation pipeline".to_string()],
        }
    }
}

impl Default for RuleBasedPlanner {
    fn default() -> Self {
        Self {
            config: PlannerConfig::default(),
        }
    }
}

impl AgentPlanner for RuleBasedPlanner {
    fn draft_plan(&self, request: &AgentRequest) -> Result<PlannerOutcome, AgentError> {
        if request.goal.trim().is_empty() {
            return Err(AgentError::invalid_request("goal cannot be empty"));
        }

        if let Some(recipe) = self.apply_intent_recipe(request) {
            return Ok(recipe);
        }

        let mut plan = AgentPlan::new(request.task_id.clone(), Self::default_title(&request.goal))
            .with_description(format!(
                "Generated by heuristics for goal: {}",
                request.goal.trim()
            ));

        let mut explanations: Vec<String> = Vec::new();
        let mut steps: Vec<AgentPlanStep> = Vec::new();

        if self.config.auto_navigate {
            if let Some(url) = extract_first_url(&request.goal) {
                let mut nav_step = AgentPlanStep::new(
                    step_id(steps.len()),
                    "Navigate to target page",
                    AgentTool {
                        kind: AgentToolKind::Navigate { url: url.clone() },
                        wait: WaitMode::Idle,
                        timeout_ms: None,
                    },
                )
                .with_detail(format!("Open the target site at {}", url));
                nav_step.validations.push(AgentValidation {
                    description: "Ensure navigation reached the intended URL".to_string(),
                    condition: AgentWaitCondition::UrlMatches(url.clone()),
                });
                explanations.push(format!("Detected URL in goal, added navigation to {}", url));
                steps.push(nav_step);
            }
        }

        let phrases = split_phrases(&request.goal);
        for phrase in phrases {
            if steps.len() >= self.config.max_steps {
                explanations.push(format!(
                    "Reached max step limit ({}), ignoring remaining instructions",
                    self.config.max_steps
                ));
                break;
            }

            if let Some(step) = phrase_to_step(&phrase, steps.len())? {
                explanations.push(format!("Mapped '{}' to {}", phrase.trim(), step.title));
                steps.push(step);
            }
        }

        if steps.is_empty() {
            return Ok(self.build_scaffold_plan(request));
        }

        for step in steps.into_iter() {
            plan.push_step(step);
        }

        if requires_weather_pipeline(request) {
            let parse_step_id = ensure_weather_parse_step(&mut plan);
            ensure_weather_deliver_step(&mut plan, &parse_step_id);
        }

        plan.meta = AgentPlanMeta {
            rationale: explanations.clone(),
            risk_assessment: default_risks(request),
            vendor_context: vendor_context_snapshot(request),
            overlays: Vec::new(),
        };

        attach_stage_metadata(&mut plan, request);
        Ok(PlannerOutcome { plan, explanations })
    }
}

fn attach_stage_metadata(plan: &mut AgentPlan, request: &AgentRequest) {
    let stage_plan = STAGE_GRAPH.plan_for_request(request);
    let stages: Vec<Value> = stage_plan
        .stages
        .iter()
        .map(|chain| {
            json!({
                "stage": chain.stage.as_str(),
                "strategies": chain.strategies,
            })
        })
        .collect();
    plan.meta.vendor_context.insert(
        "stage_plan".to_string(),
        json!({
            "plan_id": stage_plan.id,
            "stages": stages,
        }),
    );
}

fn step_id(index: usize) -> String {
    format!("step-{}", index + 1)
}

fn phrase_to_step(phrase: &str, index: usize) -> Result<Option<AgentPlanStep>, AgentError> {
    let normalized = phrase.trim();
    if normalized.is_empty() {
        return Ok(None);
    }

    let lowercase = normalized.to_lowercase();

    if let Some(seconds) = capture_wait_seconds(&lowercase) {
        let step = AgentPlanStep::new(
            step_id(index),
            "Wait for duration",
            AgentTool {
                kind: AgentToolKind::Wait {
                    condition: AgentWaitCondition::Duration(seconds * 1000),
                },
                wait: WaitMode::None,
                timeout_ms: Some(seconds * 1000 + 1000),
            },
        )
        .with_detail(format!(
            "Pause for {} seconds to allow the page to settle",
            seconds
        ));
        return Ok(Some(step));
    }

    if lowercase.starts_with("wait for") || lowercase.starts_with("wait until") {
        if let Some(label) = extract_first_quoted(normalized) {
            let locator = AgentLocator::Text {
                content: label.clone(),
                exact: false,
            };
            let mut step = AgentPlanStep::new(
                step_id(index),
                "Wait for element",
                AgentTool {
                    kind: AgentToolKind::Wait {
                        condition: AgentWaitCondition::ElementVisible(locator.clone()),
                    },
                    wait: WaitMode::None,
                    timeout_ms: None,
                },
            )
            .with_detail(format!(
                "Wait until element containing '{}' becomes visible",
                label
            ));
            step.validations.push(AgentValidation {
                description: format!("Element '{}' becomes visible", label),
                condition: AgentWaitCondition::ElementVisible(locator),
            });
            return Ok(Some(step));
        }
    }

    if lowercase.contains("click") {
        let locator = derive_click_locator(normalized).ok_or_else(|| {
            AgentError::unsupported(format!(
                "could not derive locator for instruction '{}'",
                phrase
            ))
        })?;
        let mut step = AgentPlanStep::new(
            step_id(index),
            "Click element",
            AgentTool {
                kind: AgentToolKind::Click {
                    locator: locator.clone(),
                },
                wait: WaitMode::DomReady,
                timeout_ms: None,
            },
        )
        .with_detail(format!(
            "Click element referenced by instruction '{}'.",
            normalized
        ));
        step.validations.push(AgentValidation {
            description: "Target element becomes hidden or navigates away".to_string(),
            condition: AgentWaitCondition::ElementHidden(locator.clone()),
        });
        return Ok(Some(step));
    }

    if lowercase.contains("enter")
        || lowercase.contains("type")
        || lowercase.contains("fill")
        || lowercase.contains("input")
    {
        if let Some(step) = derive_type_step(normalized, index)? {
            return Ok(Some(step));
        }
    }

    if lowercase.contains("select") {
        if let Some(step) = derive_select_step(normalized, index)? {
            return Ok(Some(step));
        }
    }

    if lowercase.contains("scroll") {
        if let Some(step) = derive_scroll_step(normalized, index)? {
            return Ok(Some(step));
        }
    }

    Ok(None)
}

fn derive_click_locator(phrase: &str) -> Option<AgentLocator> {
    if let Some(label) = extract_first_quoted(phrase) {
        return Some(AgentLocator::Text {
            content: label,
            exact: false,
        });
    }

    if phrase.to_lowercase().contains("submit") {
        return Some(AgentLocator::Text {
            content: "submit".to_string(),
            exact: false,
        });
    }

    if let Some(capture) = CLICK_WORD_REGEX.captures(phrase) {
        let word = capture.name("label")?.as_str().trim().to_string();
        if !word.is_empty() {
            return Some(AgentLocator::Text {
                content: word,
                exact: false,
            });
        }
    }

    None
}

fn derive_type_step(phrase: &str, index: usize) -> Result<Option<AgentPlanStep>, AgentError> {
    let value = extract_first_quoted(phrase).unwrap_or_else(|| infer_value(phrase));
    let field = extract_second_quoted(phrase).or_else(|| infer_field_name(phrase));

    if value.is_empty() {
        return Ok(None);
    }

    let locator = field
        .map(|name| {
            AgentLocator::Css(format!(
                "input[name='{}']",
                sanitize_selector_component(&name)
            ))
        })
        .unwrap_or_else(|| AgentLocator::Css("input".to_string()));

    let mut step = AgentPlanStep::new(
        step_id(index),
        "Type text",
        AgentTool {
            kind: AgentToolKind::TypeText {
                locator: locator.clone(),
                text: value.clone(),
                submit: should_submit(phrase),
            },
            wait: WaitMode::DomReady,
            timeout_ms: None,
        },
    )
    .with_detail(format!(
        "Type '{}' into {:?}",
        value,
        locator_description(&locator)
    ));

    step.validations.push(AgentValidation {
        description: "Field contains expected value".to_string(),
        condition: AgentWaitCondition::ElementVisible(locator.clone()),
    });

    Ok(Some(step))
}

fn derive_select_step(phrase: &str, index: usize) -> Result<Option<AgentPlanStep>, AgentError> {
    let option = extract_first_quoted(phrase).unwrap_or_default();
    if option.is_empty() {
        return Ok(None);
    }

    let field = extract_second_quoted(phrase).or_else(|| infer_field_name(phrase));
    let locator = field
        .map(|name| {
            AgentLocator::Css(format!(
                "select[name='{}']",
                sanitize_selector_component(&name)
            ))
        })
        .unwrap_or_else(|| AgentLocator::Css("select".to_string()));

    let mut step = AgentPlanStep::new(
        step_id(index),
        "Select option",
        AgentTool {
            kind: AgentToolKind::Select {
                locator: locator.clone(),
                value: option.clone(),
                method: Some("text".to_string()),
            },
            wait: WaitMode::DomReady,
            timeout_ms: None,
        },
    )
    .with_detail(format!(
        "Choose option '{}' from {:?}",
        option,
        locator_description(&locator)
    ));

    step.validations.push(AgentValidation {
        description: "Dropdown reflects selected value".to_string(),
        condition: AgentWaitCondition::ElementVisible(locator.clone()),
    });

    Ok(Some(step))
}

fn derive_scroll_step(phrase: &str, index: usize) -> Result<Option<AgentPlanStep>, AgentError> {
    let lower = phrase.to_lowercase();

    if lower.contains("top") {
        return Ok(Some(
            AgentPlanStep::new(
                step_id(index),
                "Scroll to top",
                AgentTool {
                    kind: AgentToolKind::Scroll {
                        target: AgentScrollTarget::Top,
                    },
                    wait: WaitMode::None,
                    timeout_ms: None,
                },
            )
            .with_detail("Scroll the page to the top"),
        ));
    }

    if lower.contains("bottom") {
        return Ok(Some(
            AgentPlanStep::new(
                step_id(index),
                "Scroll to bottom",
                AgentTool {
                    kind: AgentToolKind::Scroll {
                        target: AgentScrollTarget::Bottom,
                    },
                    wait: WaitMode::None,
                    timeout_ms: None,
                },
            )
            .with_detail("Scroll the page to the bottom"),
        ));
    }

    if let Some(delta) = capture_scroll_pixels(phrase) {
        return Ok(Some(build_scroll_pixels_step(index, delta)));
    }

    if lower.contains("down") {
        let delta = capture_scroll_pixels(phrase).unwrap_or(DEFAULT_SCROLL_PIXELS);
        return Ok(Some(build_scroll_pixels_step(index, delta.abs())));
    }

    if lower.contains("up") {
        let delta = capture_scroll_pixels(phrase).unwrap_or(DEFAULT_SCROLL_PIXELS);
        return Ok(Some(build_scroll_pixels_step(index, -(delta.abs()))));
    }

    Ok(None)
}

fn requires_weather_pipeline(request: &AgentRequest) -> bool {
    let mut sources = Vec::new();
    if let Some(primary) = request.intent.primary_goal.as_deref() {
        sources.push(primary);
    }
    sources.push(request.goal.as_str());
    first_weather_subject(sources.iter().copied()).is_some()
        || request
            .intent
            .required_outputs
            .iter()
            .any(|output| schema_matches_weather(&output.schema))
}

fn schema_matches_weather(schema: &str) -> bool {
    schema
        .trim()
        .trim_end_matches(".json")
        .eq_ignore_ascii_case("weather_report_v1")
}

fn ensure_weather_parse_step(plan: &mut AgentPlan) -> String {
    if let Some(existing) = plan
        .steps
        .iter()
        .find(|step| matches_weather_parse(step))
        .map(|step| step.id.clone())
    {
        return existing;
    }

    let parse_id = unique_step_id(plan, "weather-parse");
    let parse_step = AgentPlanStep::new(
        parse_id.clone(),
        "解析天气数据",
        AgentTool {
            kind: AgentToolKind::Custom {
                name: "data.parse.weather".to_string(),
                payload: json!({}),
            },
            wait: WaitMode::Idle,
            timeout_ms: None,
        },
    )
    .with_detail("解析天气小组件，输出 weather_report_v1");

    plan.push_step(parse_step);
    parse_id
}

fn ensure_weather_deliver_step(plan: &mut AgentPlan, source_step_id: &str) {
    let has_weather_deliver = plan.steps.iter().any(|step| match &step.tool.kind {
        AgentToolKind::Custom { name, payload }
            if name.eq_ignore_ascii_case("data.deliver.structured") =>
        {
            payload
                .get("schema")
                .and_then(Value::as_str)
                .map(|schema| schema.eq_ignore_ascii_case("weather_report_v1"))
                .unwrap_or(false)
        }
        _ => false,
    });

    if has_weather_deliver {
        return;
    }

    let deliver_id = unique_step_id(plan, "deliver-weather");
    let deliver_step = AgentPlanStep::new(
        deliver_id,
        "交付天气结构化结果",
        AgentTool {
            kind: AgentToolKind::Custom {
                name: "data.deliver.structured".to_string(),
                payload: json!({
                    "schema": "weather_report_v1",
                    "artifact_label": "structured.weather_report_v1",
                    "filename": "weather_report_v1.json",
                    "source_step_id": source_step_id
                }),
            },
            wait: WaitMode::Idle,
            timeout_ms: None,
        },
    )
    .with_detail("生成 weather_report_v1 并附带可读摘要");

    plan.push_step(deliver_step);
}

fn matches_weather_parse(step: &AgentPlanStep) -> bool {
    matches!(
        &step.tool.kind,
        AgentToolKind::Custom { name, .. } if name.eq_ignore_ascii_case("data.parse.weather")
    )
}

fn build_scroll_pixels_step(index: usize, delta: i32) -> AgentPlanStep {
    let direction = if delta >= 0 { "down" } else { "up" };
    AgentPlanStep::new(
        step_id(index),
        if delta >= 0 {
            "Scroll down"
        } else {
            "Scroll up"
        },
        AgentTool {
            kind: AgentToolKind::Scroll {
                target: AgentScrollTarget::Pixels(delta),
            },
            wait: WaitMode::None,
            timeout_ms: None,
        },
    )
    .with_detail(format!("Scroll {} pixels {}", delta.abs(), direction))
}

fn capture_wait_seconds(text: &str) -> Option<u64> {
    WAIT_SECONDS_REGEX
        .captures(text)
        .and_then(|capture| capture.name("secs"))
        .and_then(|m| m.as_str().parse::<u64>().ok())
}

fn extract_first_url(text: &str) -> Option<String> {
    URL_REGEX
        .find(text)
        .map(|m| m.as_str().trim_end_matches(['.', ',', ';']).to_string())
}

fn preferred_intent_site(request: &AgentRequest, fallback: &str) -> String {
    request
        .intent
        .target_sites
        .first()
        .cloned()
        .unwrap_or_else(|| fallback.to_string())
}

#[derive(Debug, Clone)]
struct QuoteIntentContext {
    metal_label: String,
    slug: String,
    contract: String,
    prefer_spot: bool,
    allowed_markets: Vec<String>,
}

impl QuoteIntentContext {
    fn from_request(request: &AgentRequest) -> Self {
        let text = if !request.goal.trim().is_empty() {
            request.goal.as_str()
        } else {
            request
                .intent
                .primary_goal
                .as_deref()
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| request.goal.as_str())
        };
        let (label, symbol) = detect_metal_symbol(text);
        let slug = symbol.to_ascii_lowercase();
        let prefer_spot =
            contains_any(text, &["现货", "spot", "即期"]) || matches!(symbol, "AG" | "AU");
        let contract_suffix = detect_contract_suffix(text).unwrap_or_else(|| "0".to_string());
        let contract = format!("{}{}", symbol, contract_suffix);
        let allowed_markets = infer_allowed_markets(text, prefer_spot);
        Self {
            metal_label: label.to_string(),
            slug,
            contract,
            prefer_spot,
            allowed_markets,
        }
    }

    fn as_query(&self) -> QuoteQuery<'_> {
        QuoteQuery {
            metal_label: &self.metal_label,
            contract: &self.contract,
            slug: &self.slug,
            prefer_spot: self.prefer_spot,
            allowed_markets: &self.allowed_markets,
        }
    }

    fn validation_keywords(&self) -> Vec<String> {
        let mut keywords = Vec::new();
        keywords.push(self.metal_label.clone());
        append_unique_keyword(&mut keywords, &self.slug);
        append_unique_keyword(&mut keywords, &self.slug.to_ascii_uppercase());
        append_unique_keyword(&mut keywords, &self.contract);
        keywords
    }
}

fn contains_any(haystack: &str, needles: &[&str]) -> bool {
    if haystack.is_empty() {
        return false;
    }
    let lower = haystack.to_ascii_lowercase();
    needles.iter().any(|needle| {
        let trimmed = needle.trim();
        if trimmed.is_empty() {
            return false;
        }
        haystack.contains(trimmed) || lower.contains(&trimmed.to_ascii_lowercase())
    })
}

fn detect_metal_symbol(text: &str) -> (&'static str, &'static str) {
    for keyword in METAL_KEYWORDS.iter() {
        if contains_any(text, keyword.aliases) {
            return (keyword.label, keyword.symbol);
        }
    }
    ("铜", "CU")
}

fn detect_contract_suffix(text: &str) -> Option<String> {
    CONTRACT_HINT_RE
        .captures(text)
        .and_then(|caps| caps.get(1))
        .map(|m| m.as_str().to_string())
        .filter(|code| code.len() >= 2)
        .map(|code| {
            if code.len() == 2 {
                format!("{:02}{}", Utc::now().year() % 100, code)
            } else {
                code
            }
        })
}

fn infer_allowed_markets(text: &str, prefer_spot: bool) -> Vec<String> {
    let mut markets = Vec::new();
    if prefer_spot {
        markets.push("上海期货交易所".to_string());
    }
    if contains_any(text, &["伦敦", "LME"]) {
        markets.push("伦敦金属交易所".to_string());
    }
    if contains_any(text, &["上海", "SHFE", "国内"]) {
        markets.push("上海期货交易所".to_string());
    }
    if markets.is_empty() {
        markets.push("上海期货交易所".to_string());
        markets.push("伦敦金属交易所".to_string());
    }
    dedup_preserve(markets)
}

fn dedup_preserve(source: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut output = Vec::new();
    for entry in source {
        if seen.insert(entry.clone()) {
            output.push(entry);
        }
    }
    output
}

struct MetalKeyword {
    label: &'static str,
    symbol: &'static str,
    aliases: &'static [&'static str],
}

const METAL_KEYWORDS: &[MetalKeyword] = &[
    MetalKeyword {
        label: "铜",
        symbol: "CU",
        aliases: &["铜", "copper", "cu"],
    },
    MetalKeyword {
        label: "铝",
        symbol: "AL",
        aliases: &["铝", "aluminum", "al"],
    },
    MetalKeyword {
        label: "镍",
        symbol: "NI",
        aliases: &["镍", "nickel", "ni"],
    },
    MetalKeyword {
        label: "锌",
        symbol: "ZN",
        aliases: &["锌", "zinc", "zn"],
    },
    MetalKeyword {
        label: "铅",
        symbol: "PB",
        aliases: &["铅", "lead", "pb"],
    },
    MetalKeyword {
        label: "锡",
        symbol: "SN",
        aliases: &["锡", "tin", "sn"],
    },
    MetalKeyword {
        label: "黄金",
        symbol: "AU",
        aliases: &["金", "黄金", "gold", "au"],
    },
    MetalKeyword {
        label: "白银",
        symbol: "AG",
        aliases: &["银", "白银", "silver", "ag"],
    },
    MetalKeyword {
        label: "原油",
        symbol: "SC",
        aliases: &["油", "原油", "oil", "sc"],
    },
];

static CONTRACT_HINT_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"(\d{4})").unwrap());

fn required_schema(request: &AgentRequest, fallback: &str) -> String {
    request
        .intent
        .required_outputs
        .first()
        .map(|output| output.schema.clone())
        .unwrap_or_else(|| fallback.to_string())
}

fn split_phrases(input: &str) -> Vec<String> {
    let mut parts = vec![input.to_string()];
    for sep in [" then ", " and then ", "->", ";", "."] {
        let mut next = Vec::new();
        for part in parts {
            for segment in part.split(sep) {
                let trimmed = segment.trim();
                if !trimmed.is_empty() {
                    next.push(trimmed.to_string());
                }
            }
        }
        parts = next;
    }
    parts
}

fn canonical_url_for_request(request: &AgentRequest) -> String {
    if let Some(site) = request.intent.target_sites.first() {
        if !site.trim().is_empty() {
            return site.clone();
        }
    }
    let goal = request
        .intent
        .primary_goal
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| request.goal.as_str())
        .trim();
    infer_url_from_goal(goal)
}

fn infer_url_from_goal(goal: &str) -> String {
    if let Some(explicit) = extract_first_url(goal) {
        return explicit;
    }

    let lowered = goal.to_ascii_lowercase();

    if lowered.contains("baidu") || goal.contains("百度") {
        return "https://www.baidu.com".to_string();
    }

    let encoded = encode_query(goal);
    if lowered.contains("github") || lowered.contains("repo") || goal.contains("仓库") {
        return format!("https://github.com/search?q={encoded}");
    }
    if lowered.contains("weather") || goal.contains("天气") {
        return format!("https://www.baidu.com/s?wd={encoded}%20天气");
    }
    if lowered.contains("news") || goal.contains("新闻") {
        return format!("https://news.baidu.com/ns?word={encoded}");
    }
    if lowered.contains("market") || lowered.contains("stock") || goal.contains("行情") {
        return format!("https://www.baidu.com/s?wd={encoded}%20行情");
    }
    format!("https://www.baidu.com/s?wd={encoded}")
}

fn first_required_schema(request: &AgentRequest) -> Option<String> {
    request.intent.required_outputs.first().map(|output| {
        output
            .schema
            .trim()
            .trim_end_matches(".json")
            .to_ascii_lowercase()
    })
}

fn encode_query(goal: &str) -> String {
    form_urlencoded::byte_serialize(goal.as_bytes()).collect()
}

fn assemble_validation_keywords(request: &AgentRequest, extras: &[String]) -> Vec<String> {
    let mut keywords = default_intent_keywords(request);
    for extra in extras {
        append_unique_keyword(&mut keywords, extra);
    }
    keywords.truncate(10);
    keywords
}

fn default_intent_keywords(request: &AgentRequest) -> Vec<String> {
    let mut keywords: Vec<String> = request
        .intent
        .validation_keywords
        .iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect();
    if keywords.is_empty() {
        if let Some(goal) = request.intent.primary_goal.as_deref() {
            keywords.extend(split_goal_terms(goal));
        }
    }
    if keywords.is_empty() {
        keywords.extend(split_goal_terms(&request.goal));
    }
    keywords
}

fn split_goal_terms(goal: &str) -> Vec<String> {
    if goal.trim().is_empty() {
        return Vec::new();
    }
    let mut terms = Vec::new();
    for chunk in goal.split(|ch: char| {
        matches!(
            ch,
            ',' | '，'
                | '。'
                | ';'
                | '；'
                | ':'
                | '：'
                | '/'
                | '|'
                | '\\'
                | '、'
                | '!'
                | '！'
                | '?'
                | '？'
                | '-'
                | '—'
                | '\t'
        )
    }) {
        let trimmed = chunk.trim();
        if !trimmed.is_empty() {
            terms.push(trimmed.to_string());
        }
    }
    if terms.is_empty() {
        terms.push(goal.trim().to_string());
    }
    terms
}

fn append_unique_keyword(keywords: &mut Vec<String>, candidate: &str) {
    let trimmed = candidate.trim();
    if trimmed.is_empty() {
        return;
    }
    if keywords
        .iter()
        .any(|existing| existing.eq_ignore_ascii_case(trimmed))
    {
        return;
    }
    keywords.push(trimmed.to_string());
}

fn collect_allowed_domains(intent_domains: &[String], urls: &[String]) -> Vec<String> {
    let mut domains: Vec<String> = intent_domains
        .iter()
        .filter_map(|value| normalize_domain(value))
        .collect();
    for url in urls {
        if let Some(domain) = normalize_domain(url) {
            domains.push(domain);
        }
    }
    domains.retain(|value| !value.is_empty());
    let mut seen = HashSet::new();
    domains
        .into_iter()
        .filter(|domain| seen.insert(domain.clone()))
        .collect()
}

fn normalize_domain(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.contains('/') {
        Url::parse(trimmed)
            .ok()
            .and_then(|parsed| parsed.domain().map(|domain| domain.to_ascii_lowercase()))
    } else {
        Some(trimmed.trim_start_matches("www.").to_ascii_lowercase())
    }
}

fn candidate_urls_from_payload(payload: &Value, navigate_url: &str) -> Vec<String> {
    let mut urls = vec![navigate_url.to_string()];
    if let Some(fallbacks) = payload.get("fallback_sources").and_then(Value::as_array) {
        for entry in fallbacks {
            if let Some(url) = entry.get("source_url").and_then(Value::as_str) {
                urls.push(url.to_string());
            }
        }
    }
    urls
}

fn build_target_validation_step(
    index: usize,
    source_step_id: &str,
    keywords: &[String],
    allowed_domains: &[String],
    expected_status: Option<u16>,
    detail: &str,
) -> Option<AgentPlanStep> {
    if keywords.is_empty() && allowed_domains.is_empty() && expected_status.is_none() {
        return None;
    }
    let payload = json!({
        "source_step_id": source_step_id,
        "keywords": keywords,
        "allowed_domains": allowed_domains,
        "expected_status": expected_status,
    });
    Some(
        AgentPlanStep::new(
            step_id(index),
            "验证目标页面",
            AgentTool {
                kind: AgentToolKind::Custom {
                    name: "data.validate-target".to_string(),
                    payload,
                },
                wait: WaitMode::None,
                timeout_ms: Some(3_000),
            },
        )
        .with_detail(detail.to_string()),
    )
}

fn extract_first_quoted(text: &str) -> Option<String> {
    QUOTED_REGEX
        .captures(text)
        .and_then(|caps| caps.get(1).or_else(|| caps.get(2)))
        .map(|m| m.as_str().to_string())
}

fn extract_second_quoted(text: &str) -> Option<String> {
    let mut iter = QUOTED_REGEX.captures_iter(text);
    iter.next()?;
    iter.next()
        .and_then(|caps| caps.get(1).or_else(|| caps.get(2)))
        .map(|m| m.as_str().to_string())
}

fn infer_value(phrase: &str) -> String {
    if let Some(capture) = VALUE_WORD_REGEX.captures(phrase) {
        capture
            .name("value")
            .map(|m| m.as_str().trim().to_string())
            .unwrap_or_default()
    } else {
        String::new()
    }
}

fn infer_field_name(phrase: &str) -> Option<String> {
    if let Some(capture) = FIELD_WORD_REGEX.captures(phrase) {
        capture
            .name("field")
            .map(|m| {
                m.as_str()
                    .trim_matches(|c: char| !c.is_alphanumeric() && c != ' ')
            })
            .map(|s| s.trim().to_string())
    } else {
        None
    }
}

fn sanitize_selector_component(input: &str) -> String {
    input
        .chars()
        .filter(|ch| ch.is_alphanumeric() || *ch == '_' || *ch == '-')
        .collect::<String>()
        .to_lowercase()
}

fn capture_scroll_pixels(text: &str) -> Option<i32> {
    let normalized = text.to_lowercase();
    SCROLL_PIXELS_REGEX
        .captures(&normalized)
        .and_then(|caps| caps.name("value"))
        .and_then(|m| m.as_str().replace(',', "").parse::<i32>().ok())
        .map(|value| {
            if normalized.contains("up") && !normalized.contains("down") {
                -value.abs()
            } else {
                value.abs()
            }
        })
}

fn locator_description(locator: &AgentLocator) -> Cow<'static, str> {
    match locator {
        AgentLocator::Css(_) => Cow::Borrowed("a CSS selector"),
        AgentLocator::Aria { .. } => Cow::Borrowed("an ARIA role/name pair"),
        AgentLocator::Text { .. } => Cow::Borrowed("text content match"),
    }
}

fn unique_step_id(plan: &AgentPlan, base: &str) -> String {
    if plan.steps.iter().all(|step| step.id != base) {
        return base.to_string();
    }
    let mut counter = 1;
    loop {
        let candidate = format!("{}-{}", base, counter);
        if plan.steps.iter().all(|step| step.id != candidate) {
            return candidate;
        }
        counter += 1;
    }
}

fn should_submit(phrase: &str) -> bool {
    let lowered = phrase.to_lowercase();
    lowered.contains("submit")
        || lowered.contains("press enter")
        || lowered.contains("hit enter")
        || lowered.contains("press return")
}

fn default_risks(request: &AgentRequest) -> Vec<String> {
    let mut risks = Vec::new();
    if request.goal.to_lowercase().contains("login") {
        risks
            .push("Contains authentication instructions; confirm credential handling.".to_string());
    }
    if request.goal.to_lowercase().contains("payment") {
        risks.push("Potential payment flow; ensure approval before executing.".to_string());
    }
    if risks.is_empty() {
        risks.push("Standard automation risk; monitor for navigation drift.".to_string());
    }
    risks
}

fn recipe_meta(request: &AgentRequest, recipe: &str) -> AgentPlanMeta {
    let mut vendor_context = vendor_context_snapshot(request);
    vendor_context.insert(
        "intent_recipe".to_string(),
        Value::String(recipe.to_string()),
    );
    AgentPlanMeta {
        rationale: vec![format!("Intent recipe {recipe} applied")],
        risk_assessment: vec![format!("Template workflow: {recipe}")],
        vendor_context,
        overlays: Vec::new(),
    }
}

fn vendor_context_snapshot(request: &AgentRequest) -> HashMap<String, Value> {
    let mut map = HashMap::new();
    map.insert(
        "conversation_turns".to_string(),
        Value::Number(JsonNumber::from(request.conversation.len() as u64)),
    );
    map.insert(
        "constraints".to_string(),
        Value::Array(
            request
                .constraints
                .iter()
                .map(|c| Value::String(c.clone()))
                .collect(),
        ),
    );
    map
}

static URL_REGEX: Lazy<Regex> = Lazy::new(|| Regex::new(r"https?://[^\s]+").expect("url regex"));
static QUOTED_REGEX: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#""([^"]+)"|'([^']+)'"#).expect("quoted regex"));
static WAIT_SECONDS_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"wait(?:\s+for)?\s+(?P<secs>\d+)(?:\s*)(?:seconds|secs|s)").expect("wait regex")
});
static CLICK_WORD_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"click(?: on)?(?: the)? (?P<label>[a-zA-Z0-9 ]+)").expect("click regex")
});
static VALUE_WORD_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?:type|enter|input) (?P<value>[a-zA-Z0-9@.\-_]+)").expect("value regex")
});
static FIELD_WORD_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?:into|in|for) (?:the )?(?P<field>[a-zA-Z0-9 _-]+)(?: field)?")
        .expect("field regex")
});
static SCROLL_PIXELS_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"scroll(?:\s+(?:up|down))?(?:\s+by)?\s+(?P<value>[-+]?[0-9]{1,4})(?:\s*(?:px|pixels)?)",
    )
    .expect("scroll pixels regex")
});

const DEFAULT_SCROLL_PIXELS: i32 = 600;

#[cfg(test)]
mod tests {
    use super::*;
    use soulbrowser_core_types::TaskId;

    fn build_quote_request(goal: &str) -> AgentRequest {
        let mut request = AgentRequest::new(TaskId::new(), goal);
        request.intent.intent_id = Some("market_quote_lookup".to_string());
        request.intent.primary_goal = Some(goal.to_string());
        request
    }

    #[test]
    fn quote_source_defaults_to_qh_contract_page() {
        let request = build_quote_request("查询铜价");
        let context = QuoteIntentContext::from_request(&request);
        let plan_spec = resolve_quote_plan(&request, context.as_query());
        assert!(plan_spec
            .navigate_url
            .starts_with("https://quote.eastmoney.com/qh/"));
    }

    #[test]
    fn quote_source_prefers_spot_site_when_requested() {
        let request = build_quote_request("查询铜现货价格");
        let context = QuoteIntentContext::from_request(&request);
        assert_eq!(context.prefer_spot, true);
        let plan_spec = resolve_quote_plan(&request, context.as_query());
        assert!(plan_spec
            .navigate_url
            .starts_with("https://finance.sina.com.cn/futures/quotes/"));
    }

    #[test]
    fn silver_quote_defaults_to_spot_site() {
        let request = build_quote_request("查询白银价格");
        let context = QuoteIntentContext::from_request(&request);
        assert_eq!(context.slug, "ag");
        assert!(context.prefer_spot);
        let plan_spec = resolve_quote_plan(&request, context.as_query());
        assert_eq!(
            plan_spec.navigate_url,
            "https://finance.sina.com.cn/futures/quotes/AG0.shtml"
        );
    }

    #[test]
    fn quote_source_respects_explicit_target_site() {
        let mut request = build_quote_request("查询铜价");
        request.intent.target_sites = vec!["https://example.com/custom".to_string()];
        let context = QuoteIntentContext::from_request(&request);
        let plan_spec = resolve_quote_plan(&request, context.as_query());
        assert_eq!(plan_spec.navigate_url, "https://example.com/custom");
    }

    #[test]
    fn quote_plan_contains_fallback_sources() {
        let request = build_quote_request("查询铜价");
        let context = QuoteIntentContext::from_request(&request);
        let plan_spec = resolve_quote_plan(&request, context.as_query());
        assert!(plan_spec
            .payload
            .get("fallback_sources")
            .and_then(Value::as_array)
            .map(|arr| !arr.is_empty())
            .unwrap_or(false));
    }

    #[test]
    fn quote_context_prefers_user_goal_over_template_goal() {
        let mut request = build_quote_request("请查看白银最新价格");
        request.intent.primary_goal = Some("查询指定金属的最新行情".to_string());
        let context = QuoteIntentContext::from_request(&request);
        assert_eq!(context.metal_label, "白银");
        assert_eq!(context.contract, "AG0");
    }

    #[test]
    fn quote_plan_ignores_hint_sites_for_manual_override() {
        let mut request = build_quote_request("查一下白银价格");
        request.intent.primary_goal = Some("查询指定金属的最新行情".to_string());
        request.intent.target_sites = vec!["https://quote.eastmoney.com/qh/CU0.html".to_string()];
        request.intent.target_sites_are_hints = true;
        let context = QuoteIntentContext::from_request(&request);
        let plan_spec = resolve_quote_plan(&request, context.as_query());
        assert!(plan_spec.navigate_url.contains("AG0"));
        assert!(plan_spec.sources.iter().all(|id| id != "manual_override"));
    }

    #[test]
    fn mismatched_manual_override_is_demoted() {
        let mut request = build_quote_request("查一下白银价格");
        request.intent.target_sites = vec!["https://quote.eastmoney.com/qh/CU0.html".to_string()];
        request.intent.target_sites_are_hints = false;
        let context = QuoteIntentContext::from_request(&request);
        let plan_spec = resolve_quote_plan(&request, context.as_query());
        assert!(plan_spec.navigate_url.to_ascii_lowercase().contains("ag"));
        assert!(plan_spec.sources.iter().all(|id| id != "manual_override"));
    }
}

fn auto_act_domain_hints(
    target_domain: Option<String>,
    guardrail_domains: &[String],
) -> Vec<String> {
    let mut hints = Vec::new();
    let mut seen = HashSet::new();
    if let Some(domain) = target_domain {
        let normalized = domain.trim().trim_start_matches("www.");
        if !normalized.is_empty() && seen.insert(normalized.to_ascii_lowercase()) {
            hints.push(normalized.to_string());
        }
    }
    for domain in guardrail_domains {
        let normalized = domain.trim().trim_start_matches("www.");
        if normalized.is_empty() {
            continue;
        }
        if seen.insert(normalized.to_ascii_lowercase()) {
            hints.push(normalized.to_string());
        }
    }
    hints
}

fn guardrail_refresh_queries(base_query: &str, domains: &[String]) -> Vec<String> {
    let mut queries = Vec::new();
    let mut seen = HashSet::new();
    for domain in domains {
        let candidate = format!("{base_query} site:{domain}");
        if seen.insert(candidate.to_ascii_lowercase()) {
            queries.push(candidate);
        }
    }
    if seen.insert(base_query.to_ascii_lowercase()) {
        queries.push(base_query.to_string());
    }
    let fallback = format!("{base_query} 最新 行情");
    if seen.insert(fallback.to_ascii_lowercase()) {
        queries.push(fallback);
    }
    queries.truncate(5);
    queries
}

fn guardrail_search_query(base_query: &str, domains: &[String]) -> String {
    if let Some(domain) = domains.first() {
        format!("{base_query} site:{domain}")
    } else {
        base_query.to_string()
    }
}
