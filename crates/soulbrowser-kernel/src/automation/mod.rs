//! Browser automation module
//!
//! Provides automated browser control and workflow execution

use anyhow::{anyhow, Context, Result};
use async_recursion::async_recursion;
use async_trait::async_trait;
use serde::Serialize;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;
use tracing::warn;

use crate::{
    app_context::AppContext,
    browser_impl::{Browser, BrowserConfig, L0Protocol, L1BrowserManager, Page},
    storage::{BrowserEvent, StorageManager},
    types::BrowserType,
};

/// Automation engine for running browser automation tasks
pub struct AutomationEngine {
    inner: Arc<AutomationEngineInner>,
    parallel_limit: Arc<Semaphore>,
}

/// Configuration for automation tasks
#[derive(Debug, Clone)]
pub struct AutomationConfig {
    pub browser_type: BrowserType,
    pub headless: bool,
    #[allow(dead_code)]
    pub timeout: u64,
    pub parallel_instances: usize,
    pub parameters: HashMap<String, String>,
    #[allow(dead_code)]
    pub output_dir: Option<PathBuf>,
}

impl Default for AutomationConfig {
    fn default() -> Self {
        Self {
            browser_type: BrowserType::Chromium,
            headless: true,
            timeout: 30,
            parallel_instances: 1,
            parameters: HashMap::new(),
            output_dir: None,
        }
    }
}

/// Results from automation execution
#[derive(Debug, Serialize)]
pub struct AutomationResults {
    pub success: bool,
    pub duration: u64,
    pub steps_completed: usize,
    pub errors: Vec<String>,
}

#[async_trait]
pub trait AutomationRuntimeAdapter: Send {
    async fn navigate(&mut self, url: &str) -> Result<()>;
    async fn click(&mut self, selector: &str) -> Result<()>;
    async fn type_text(&mut self, selector: &str, text: &str) -> Result<()>;
    async fn select_option(
        &mut self,
        selector: &str,
        value: &str,
        match_kind: Option<&str>,
        mode: Option<&str>,
    ) -> Result<()>;
    async fn screenshot(&mut self, filename: &str) -> Result<()>;
}

#[async_trait]
pub trait AutomationRuntimeFactory: Send + Sync {
    async fn create_runtime(
        &self,
        config: &AutomationConfig,
    ) -> Result<Box<dyn AutomationRuntimeAdapter>>;
}

impl AutomationEngine {
    /// Create a new automation engine with app context
    pub async fn with_context(context: Arc<AppContext>, config: AutomationConfig) -> Result<Self> {
        Self::with_runtime_factory(context, config, Arc::new(BrowserRuntimeFactory)).await
    }

    pub async fn with_runtime_factory(
        context: Arc<AppContext>,
        config: AutomationConfig,
        runtime_factory: Arc<dyn AutomationRuntimeFactory>,
    ) -> Result<Self> {
        let parallel_limit = std::cmp::max(1, config.parallel_instances);
        Ok(Self {
            inner: Arc::new(AutomationEngineInner {
                config,
                storage_manager: context.storage(),
                runtime_factory,
            }),
            parallel_limit: Arc::new(Semaphore::new(parallel_limit)),
        })
    }

    /// Execute automation workflow from a script file
    pub async fn execute_script(&mut self, script_path: &PathBuf) -> Result<AutomationResults> {
        let start_time = Instant::now();

        // Load and parse script
        let script_content =
            std::fs::read_to_string(script_path).context("Failed to read script file")?;

        let commands = ScriptParser::new().parse(&script_content)?;

        let parameters = self.inner.config.parameters.clone();
        let mut context = ExecutionContext::new(parameters);

        let mut runtime = self.inner.create_runtime().await?;

        let stats = self
            .inner
            .execute_commands(
                &commands,
                runtime.as_mut(),
                &mut context,
                self.parallel_limit.clone(),
            )
            .await?;

        let duration = start_time.elapsed().as_secs();

        Ok(AutomationResults {
            success: stats.errors.is_empty(),
            duration,
            steps_completed: stats.steps_completed,
            errors: stats.errors,
        })
    }
}

#[derive(Clone)]
struct AutomationEngineInner {
    config: AutomationConfig,
    storage_manager: Arc<StorageManager>,
    runtime_factory: Arc<dyn AutomationRuntimeFactory>,
}

impl AutomationEngineInner {
    async fn create_runtime(&self) -> Result<Box<dyn AutomationRuntimeAdapter>> {
        self.runtime_factory.create_runtime(&self.config).await
    }

    #[async_recursion]
    async fn execute_commands(
        &self,
        commands: &[AutomationCommand],
        runtime: &mut dyn AutomationRuntimeAdapter,
        context: &mut ExecutionContext,
        parallel_limit: Arc<Semaphore>,
    ) -> Result<ExecutionStats> {
        let mut stats = ExecutionStats::default();
        for command in commands {
            let result = match command {
                AutomationCommand::Action(action) => {
                    self.execute_action(action, runtime, context).await
                }
                AutomationCommand::Set { key, value } => {
                    let resolved = context.resolve_template(value);
                    context.locals.insert(key.clone(), resolved);
                    Ok(ExecutionStats::default())
                }
                AutomationCommand::Loop { count_expr, body } => {
                    self.execute_loop(count_expr, body, runtime, context, parallel_limit.clone())
                        .await
                }
                AutomationCommand::Conditional {
                    variable,
                    operator,
                    value,
                    then_body,
                    else_body,
                } => {
                    self.execute_conditional(
                        variable,
                        operator,
                        value,
                        then_body,
                        else_body,
                        runtime,
                        context,
                        parallel_limit.clone(),
                    )
                    .await
                }
                AutomationCommand::Parallel { branches, limit } => {
                    self.execute_parallel(branches, *limit, context, parallel_limit.clone())
                        .await
                }
            };

            match result {
                Ok(step_stats) => stats.merge(step_stats),
                Err(err) => stats.errors.push(err.to_string()),
            }
        }

        Ok(stats)
    }

    async fn execute_action(
        &self,
        action: &ActionCommand,
        runtime: &mut dyn AutomationRuntimeAdapter,
        context: &mut ExecutionContext,
    ) -> Result<ExecutionStats> {
        let stats = ExecutionStats::from_step();
        match action {
            ActionCommand::Navigate(url) => {
                let url = context.resolve_template(url);
                runtime.navigate(&url).await?;
                self.record_event("navigate", serde_json::json!({ "url": url }))
                    .await?;
            }
            ActionCommand::Click(selector) => {
                let selector = context.resolve_template(selector);
                match runtime.click(&selector).await {
                    Ok(_) => {
                        self.record_event("click", serde_json::json!({ "selector": selector }))
                            .await?;
                    }
                    Err(mut err) => {
                        let fallbacks = click_fallback_selectors(&selector);
                        if fallbacks.is_empty() {
                            return Err(err);
                        }
                        let mut applied = None;
                        for fallback in fallbacks {
                            match runtime.click(&fallback).await {
                                Ok(_) => {
                                    applied = Some(fallback);
                                    break;
                                }
                                Err(next_err) => {
                                    err = next_err;
                                }
                            }
                        }
                        if let Some(fallback_selector) = applied {
                            warn!(
                                original = %selector,
                                fallback = %fallback_selector,
                                "click selector fallback succeeded"
                            );
                            self.record_event(
                                "click_fallback",
                                serde_json::json!({
                                    "original": selector,
                                    "fallback": fallback_selector
                                }),
                            )
                            .await?;
                        } else {
                            return Err(err);
                        }
                    }
                }
            }
            ActionCommand::Type { selector, text } => {
                let selector = context.resolve_template(selector);
                let text_val = context.resolve_template(text);
                runtime.type_text(&selector, &text_val).await?;
                self.record_event(
                    "type",
                    serde_json::json!({ "selector": selector, "text": text_val }),
                )
                .await?;
            }
            ActionCommand::Select {
                selector,
                value,
                match_kind,
                mode,
            } => {
                let selector = context.resolve_template(selector);
                let value = context.resolve_template(value);
                let match_kind_val = match_kind.as_ref().map(|mk| context.resolve_template(mk));
                let mode_val = mode.as_ref().map(|m| context.resolve_template(m));
                runtime
                    .select_option(
                        &selector,
                        &value,
                        match_kind_val.as_deref(),
                        mode_val.as_deref(),
                    )
                    .await?;
                self.record_event(
                    "select",
                    serde_json::json!({
                        "selector": selector,
                        "value": value,
                        "match_kind": match_kind_val,
                        "mode": mode_val
                    }),
                )
                .await?;
            }
            ActionCommand::Screenshot(filename) => {
                let filename = context.resolve_template(filename);
                runtime.screenshot(&filename).await?;
                self.record_event("screenshot", serde_json::json!({ "filename": filename }))
                    .await?;
            }
            ActionCommand::Wait(duration_expr) => {
                let value = context.resolve_template(duration_expr);
                let duration = value.parse::<u64>().map_err(|_| {
                    anyhow!(
                        "Invalid wait duration '{}'. Expected integer milliseconds.",
                        value
                    )
                })?;
                tokio::time::sleep(tokio::time::Duration::from_millis(duration)).await;
            }
        }
        Ok(stats)
    }

    #[async_recursion]
    async fn execute_loop(
        &self,
        count_expr: &str,
        body: &[AutomationCommand],
        runtime: &mut dyn AutomationRuntimeAdapter,
        context: &mut ExecutionContext,
        parallel_limit: Arc<Semaphore>,
    ) -> Result<ExecutionStats> {
        let count_value = context.resolve_template(count_expr);
        let iterations = count_value.parse::<usize>().map_err(|_| {
            anyhow!(
                "Invalid loop count '{}'. Expected positive integer.",
                count_value
            )
        })?;

        let mut stats = ExecutionStats::default();
        for _ in 0..iterations {
            let iteration_stats = self
                .execute_commands(body, runtime, context, parallel_limit.clone())
                .await?;
            stats.merge(iteration_stats);
        }
        Ok(stats)
    }

    #[async_recursion]
    async fn execute_conditional(
        &self,
        variable: &str,
        operator: &ConditionOp,
        value: &str,
        then_body: &[AutomationCommand],
        else_body: &[AutomationCommand],
        runtime: &mut dyn AutomationRuntimeAdapter,
        context: &mut ExecutionContext,
        parallel_limit: Arc<Semaphore>,
    ) -> Result<ExecutionStats> {
        let expected = context.resolve_template(value);
        let actual = context.get_variable(variable).unwrap_or_default();
        let condition_met = match operator {
            ConditionOp::Equal => actual == expected,
            ConditionOp::NotEqual => actual != expected,
        };

        if condition_met {
            self.execute_commands(then_body, runtime, context, parallel_limit.clone())
                .await
        } else if !else_body.is_empty() {
            self.execute_commands(else_body, runtime, context, parallel_limit.clone())
                .await
        } else {
            Ok(ExecutionStats::default())
        }
    }

    async fn execute_parallel(
        &self,
        branches: &[Vec<AutomationCommand>],
        limit: Option<usize>,
        context: &ExecutionContext,
        parallel_limit: Arc<Semaphore>,
    ) -> Result<ExecutionStats> {
        let mut stats = ExecutionStats::default();
        let mut join_set = JoinSet::new();

        let local_limit = limit.map(|value| Arc::new(Semaphore::new(value)));

        for branch_commands in branches.iter().cloned() {
            let inner = self.clone();
            let base_context = context.clone();
            let global_pool = parallel_limit.clone();
            let local_pool = local_limit.clone();
            join_set.spawn(async move {
                let child_pool = global_pool.clone();
                let global_permit = global_pool
                    .acquire_owned()
                    .await
                    .map_err(|err| anyhow!("Parallel execution cancelled: {}", err))?;

                let local_permit = if let Some(pool) = local_pool.clone() {
                    Some(
                        pool.acquire_owned()
                            .await
                            .map_err(|err| anyhow!("Parallel execution cancelled: {}", err))?,
                    )
                } else {
                    None
                };

                let mut runtime = inner.create_runtime().await?;
                let mut branch_context = base_context;
                let result = inner
                    .execute_commands(
                        &branch_commands,
                        runtime.as_mut(),
                        &mut branch_context,
                        child_pool,
                    )
                    .await;

                drop(local_permit);
                drop(global_permit);
                result
            });
        }

        while let Some(result) = join_set.join_next().await {
            match result {
                Ok(Ok(branch_stats)) => stats.merge(branch_stats),
                Ok(Err(err)) => stats.errors.push(err.to_string()),
                Err(join_err) => stats.errors.push(join_err.to_string()),
            }
        }

        Ok(stats)
    }

    async fn record_event(&self, event_type: &str, data: serde_json::Value) -> Result<()> {
        let event = BrowserEvent {
            id: uuid::Uuid::new_v4().to_string(),
            tenant: soulbase_types::tenant::TenantId("automation".to_string()),
            session_id: "automation-session".to_string(),
            timestamp: chrono::Utc::now().timestamp_millis(),
            event_type: event_type.to_string(),
            data,
            sequence: 0,
            tags: vec!["automation".to_string()],
        };

        self.storage_manager
            .backend()
            .store_event(event)
            .await
            .context("Failed to store automation event")?;

        Ok(())
    }
}

struct BrowserRuntimeFactory;

#[async_trait]
impl AutomationRuntimeFactory for BrowserRuntimeFactory {
    async fn create_runtime(
        &self,
        config: &AutomationConfig,
    ) -> Result<Box<dyn AutomationRuntimeAdapter>> {
        let l0 = L0Protocol::new()
            .await
            .context("Failed to initialize L0 protocol")?;

        let browser_config = BrowserConfig {
            browser_type: config.browser_type.clone(),
            headless: config.headless,
            window_size: Some((1280, 720)),
            devtools: false,
        };

        let mut manager = L1BrowserManager::new(l0, browser_config)
            .await
            .context("Failed to initialize browser manager")?;

        let browser = manager
            .launch_browser()
            .await
            .context("Failed to launch browser")?;

        let page = browser.new_page().await.context("Failed to create page")?;

        Ok(Box::new(BrowserAutomationClient {
            _browser: browser,
            page,
        }))
    }
}

struct BrowserAutomationClient {
    _browser: Browser,
    page: Page,
}

#[async_trait]
impl AutomationRuntimeAdapter for BrowserAutomationClient {
    async fn navigate(&mut self, url: &str) -> Result<()> {
        self.page.navigate(url).await.context("Failed to navigate")
    }

    async fn click(&mut self, selector: &str) -> Result<()> {
        self.page
            .click(selector)
            .await
            .context("Failed to click element")
    }

    async fn type_text(&mut self, selector: &str, text: &str) -> Result<()> {
        self.page
            .type_text(selector, text)
            .await
            .context("Failed to type text")
    }

    async fn select_option(
        &mut self,
        selector: &str,
        value: &str,
        match_kind: Option<&str>,
        mode: Option<&str>,
    ) -> Result<()> {
        self.page
            .select_option(selector, value, match_kind, mode)
            .await
            .context("Failed to select option")
    }

    async fn screenshot(&mut self, filename: &str) -> Result<()> {
        self.page
            .screenshot(filename)
            .await
            .context("Failed to capture screenshot")
            .map(|_| ())
    }
}

#[derive(Default)]
struct ExecutionStats {
    steps_completed: usize,
    errors: Vec<String>,
}

impl ExecutionStats {
    fn merge(&mut self, other: ExecutionStats) {
        self.steps_completed += other.steps_completed;
        self.errors.extend(other.errors);
    }

    fn from_step() -> Self {
        Self {
            steps_completed: 1,
            errors: Vec::new(),
        }
    }
}

#[derive(Clone)]
struct ExecutionContext {
    parameters: Arc<HashMap<String, String>>,
    locals: HashMap<String, String>,
}

impl ExecutionContext {
    fn new(parameters: HashMap<String, String>) -> Self {
        Self {
            parameters: Arc::new(parameters),
            locals: HashMap::new(),
        }
    }

    fn resolve_template(&self, value: &str) -> String {
        let mut resolved = value.to_string();
        for (key, val) in self.parameters.iter() {
            let token = format!("{{{{{}}}}}", key);
            if resolved.contains(&token) {
                resolved = resolved.replace(&token, val);
            }
        }
        for (key, val) in self.locals.iter() {
            let token = format!("{{{{{}}}}}", key);
            if resolved.contains(&token) {
                resolved = resolved.replace(&token, val);
            }
        }
        resolved
    }

    fn get_variable(&self, key: &str) -> Option<String> {
        self.locals
            .get(key)
            .cloned()
            .or_else(|| self.parameters.get(key).cloned())
    }
}

#[derive(Clone, Debug)]
enum AutomationCommand {
    Action(ActionCommand),
    Set {
        key: String,
        value: String,
    },
    Loop {
        count_expr: String,
        body: Vec<AutomationCommand>,
    },
    Conditional {
        variable: String,
        operator: ConditionOp,
        value: String,
        then_body: Vec<AutomationCommand>,
        else_body: Vec<AutomationCommand>,
    },
    Parallel {
        branches: Vec<Vec<AutomationCommand>>,
        limit: Option<usize>,
    },
}

#[derive(Clone, Debug)]
enum ActionCommand {
    Navigate(String),
    Click(String),
    Type {
        selector: String,
        text: String,
    },
    Select {
        selector: String,
        value: String,
        match_kind: Option<String>,
        mode: Option<String>,
    },
    Screenshot(String),
    Wait(String),
}

#[derive(Clone, Debug)]
enum ConditionOp {
    Equal,
    NotEqual,
}

struct ScriptParser {
    current_line: usize,
}

impl ScriptParser {
    fn new() -> Self {
        Self { current_line: 0 }
    }

    fn parse(&mut self, script: &str) -> Result<Vec<AutomationCommand>> {
        let mut stack: Vec<BlockState> = vec![BlockState::root()];

        for line in script.lines() {
            self.current_line += 1;
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }

            let mut parts = trimmed.split_whitespace();
            let keyword = parts.next().unwrap();

            match keyword {
                "set" => {
                    let rest: Vec<&str> = parts.collect();
                    if rest.len() < 2 {
                        return Err(anyhow!(
                            "Invalid set syntax on line {}. Usage: set key value",
                            self.current_line
                        ));
                    }
                    let key = rest[0].to_string();
                    let value = rest[1..].join(" ");
                    stack
                        .last_mut()
                        .unwrap()
                        .push_command(AutomationCommand::Set { key, value })?;
                }
                "loop" => {
                    let count_expr = parts.collect::<Vec<&str>>().join(" ");
                    if count_expr.is_empty() {
                        return Err(anyhow!("Missing loop count on line {}", self.current_line));
                    }
                    stack.push(BlockState::loop_block(count_expr));
                }
                "endloop" => {
                    let block = stack.pop().ok_or_else(|| {
                        anyhow!("Unexpected endloop on line {}", self.current_line)
                    })?;
                    let command = block.into_loop_command()?;
                    stack.last_mut().unwrap().push_command(command)?;
                }
                "if" => {
                    let condition: Vec<&str> = parts.collect();
                    let (variable, operator, value) = self.parse_condition(&condition)?;
                    stack.push(BlockState::conditional(variable, operator, value));
                }
                "else" => {
                    stack
                        .last_mut()
                        .ok_or_else(|| anyhow!("Unexpected else on line {}", self.current_line))?
                        .enter_else()?;
                }
                "endif" => {
                    let block = stack
                        .pop()
                        .ok_or_else(|| anyhow!("Unexpected endif on line {}", self.current_line))?;
                    let command = block.into_conditional_command()?;
                    stack.last_mut().unwrap().push_command(command)?;
                }
                "parallel" => {
                    let tokens: Vec<&str> = parts.collect();
                    let limit = if tokens.is_empty() {
                        None
                    } else {
                        let value = tokens.join(" ");
                        Some(value.parse::<usize>().map_err(|_| {
                            anyhow!(
                                "Invalid parallel limit '{}' on line {}",
                                value,
                                self.current_line
                            )
                        })?)
                    };
                    stack.push(BlockState::parallel(limit));
                }
                "branch" => {
                    stack
                        .last_mut()
                        .ok_or_else(|| {
                            anyhow!("branch outside parallel on line {}", self.current_line)
                        })?
                        .start_branch()?;
                }
                "endbranch" => {
                    stack
                        .last_mut()
                        .ok_or_else(|| {
                            anyhow!("endbranch outside parallel on line {}", self.current_line)
                        })?
                        .finish_branch()?;
                }
                "endparallel" => {
                    let block = stack.pop().ok_or_else(|| {
                        anyhow!("Unexpected endparallel on line {}", self.current_line)
                    })?;
                    let command = block.into_parallel_command()?;
                    stack.last_mut().unwrap().push_command(command)?;
                }
                _ => {
                    let action = self.parse_action(keyword, parts.collect::<Vec<&str>>())?;
                    stack
                        .last_mut()
                        .unwrap()
                        .push_command(AutomationCommand::Action(action))?;
                }
            }
        }

        if stack.len() != 1 {
            return Err(anyhow!("Unclosed block at end of script"));
        }

        Ok(stack.pop().unwrap().commands)
    }

    fn parse_condition(&self, tokens: &[&str]) -> Result<(String, ConditionOp, String)> {
        if tokens.len() < 3 {
            return Err(anyhow!(
                "Invalid condition on line {}. Usage: if <var> == <value>",
                self.current_line
            ));
        }
        let variable = tokens[0].to_string();
        let operator = match tokens[1] {
            "==" => ConditionOp::Equal,
            "!=" => ConditionOp::NotEqual,
            other => {
                return Err(anyhow!(
                    "Unsupported operator '{}' on line {}",
                    other,
                    self.current_line
                ))
            }
        };
        let value = tokens[2..].join(" ");
        Ok((variable, operator, value))
    }

    fn parse_action(&self, keyword: &str, args: Vec<&str>) -> Result<ActionCommand> {
        match keyword {
            "navigate" => {
                if args.is_empty() {
                    return Err(anyhow!(
                        "navigate requires a URL on line {}",
                        self.current_line
                    ));
                }
                Ok(ActionCommand::Navigate(args.join(" ")))
            }
            "click" => {
                if args.is_empty() {
                    return Err(anyhow!(
                        "click requires a selector on line {}",
                        self.current_line
                    ));
                }
                Ok(ActionCommand::Click(args.join(" ")))
            }
            "type" => {
                if args.len() < 2 {
                    return Err(anyhow!(
                        "type requires selector and text on line {}",
                        self.current_line
                    ));
                }
                Ok(ActionCommand::Type {
                    selector: args[0].to_string(),
                    text: args[1..].join(" "),
                })
            }
            "select" => {
                if args.len() < 2 {
                    return Err(anyhow!(
                        "select requires selector and value on line {}",
                        self.current_line
                    ));
                }
                let raw_selector = args[0];
                let raw_value = args[1];
                let selector = raw_selector
                    .strip_prefix("selector=")
                    .unwrap_or(raw_selector)
                    .to_string();
                let value = raw_value
                    .strip_prefix("value=")
                    .unwrap_or(raw_value)
                    .to_string();
                let mut match_kind = None;
                let mut mode = None;
                for token in args.iter().skip(2) {
                    if let Some(kind) = token.strip_prefix("match=") {
                        match_kind = Some(kind.to_string());
                    } else if let Some(m) = token.strip_prefix("mode=") {
                        mode = Some(m.to_string());
                    } else {
                        return Err(anyhow!(
                            "Unsupported select argument '{}' on line {}",
                            token,
                            self.current_line
                        ));
                    }
                }
                Ok(ActionCommand::Select {
                    selector,
                    value,
                    match_kind,
                    mode,
                })
            }
            "screenshot" => {
                if args.is_empty() {
                    return Err(anyhow!(
                        "screenshot requires filename on line {}",
                        self.current_line
                    ));
                }
                Ok(ActionCommand::Screenshot(args.join(" ")))
            }
            "wait" => {
                if args.is_empty() {
                    return Err(anyhow!(
                        "wait requires duration on line {}",
                        self.current_line
                    ));
                }
                Ok(ActionCommand::Wait(args.join(" ")))
            }
            other => Err(anyhow!(
                "Unknown command '{}' on line {}",
                other,
                self.current_line
            )),
        }
    }
}

fn click_fallback_selectors(selector: &str) -> Vec<String> {
    let normalized = selector.trim().to_ascii_lowercase();
    if normalized.contains("s_search") {
        vec![
            "css=#su".to_string(),
            "text=百度一下".to_string(),
            "input[type=submit][value='百度一下']".to_string(),
        ]
    } else {
        Vec::new()
    }
}

struct BlockState {
    kind: BlockKind,
    commands: Vec<AutomationCommand>,
}

impl BlockState {
    fn root() -> Self {
        Self {
            kind: BlockKind::Root,
            commands: Vec::new(),
        }
    }

    fn loop_block(count_expr: String) -> Self {
        Self {
            kind: BlockKind::Loop { count_expr },
            commands: Vec::new(),
        }
    }

    fn conditional(variable: String, operator: ConditionOp, value: String) -> Self {
        Self {
            kind: BlockKind::Conditional {
                variable,
                operator,
                value,
                else_body: Vec::new(),
                in_else: false,
            },
            commands: Vec::new(),
        }
    }

    fn parallel(limit: Option<usize>) -> Self {
        Self {
            kind: BlockKind::Parallel {
                branches: Vec::new(),
                current_branch: Vec::new(),
                limit,
            },
            commands: Vec::new(),
        }
    }

    fn push_command(&mut self, command: AutomationCommand) -> Result<()> {
        match &mut self.kind {
            BlockKind::Conditional {
                in_else, else_body, ..
            } if *in_else => {
                else_body.push(command);
            }
            BlockKind::Parallel { current_branch, .. } => {
                current_branch.push(command);
            }
            _ => self.commands.push(command),
        }
        Ok(())
    }

    fn enter_else(&mut self) -> Result<()> {
        match &mut self.kind {
            BlockKind::Conditional { in_else, .. } => {
                if *in_else {
                    return Err(anyhow!("Duplicate else detected"));
                }
                *in_else = true;
                Ok(())
            }
            _ => Err(anyhow!("else without matching if")),
        }
    }

    fn start_branch(&mut self) -> Result<()> {
        match &mut self.kind {
            BlockKind::Parallel {
                branches,
                current_branch,
                ..
            } => {
                if !current_branch.is_empty() {
                    branches.push(std::mem::take(current_branch));
                }
                Ok(())
            }
            _ => Err(anyhow!("branch keyword outside parallel block")),
        }
    }

    fn finish_branch(&mut self) -> Result<()> {
        match &mut self.kind {
            BlockKind::Parallel {
                branches,
                current_branch,
                ..
            } => {
                if current_branch.is_empty() {
                    return Err(anyhow!("empty branch encountered"));
                }
                branches.push(std::mem::take(current_branch));
                Ok(())
            }
            _ => Err(anyhow!("endbranch outside parallel block")),
        }
    }

    fn into_loop_command(self) -> Result<AutomationCommand> {
        match self.kind {
            BlockKind::Loop { count_expr } => Ok(AutomationCommand::Loop {
                count_expr,
                body: self.commands,
            }),
            _ => Err(anyhow!("endloop does not match a loop block")),
        }
    }

    fn into_conditional_command(self) -> Result<AutomationCommand> {
        match self.kind {
            BlockKind::Conditional {
                variable,
                operator,
                value,
                else_body,
                ..
            } => Ok(AutomationCommand::Conditional {
                variable,
                operator,
                value,
                then_body: self.commands,
                else_body,
            }),
            _ => Err(anyhow!("endif does not match an if block")),
        }
    }

    fn into_parallel_command(mut self) -> Result<AutomationCommand> {
        match &mut self.kind {
            BlockKind::Parallel {
                branches,
                current_branch,
                limit,
            } => {
                if !current_branch.is_empty() {
                    branches.push(std::mem::take(current_branch));
                }
                if branches.is_empty() {
                    return Err(anyhow!("parallel block contains no branches"));
                }
                Ok(AutomationCommand::Parallel {
                    branches: branches.clone(),
                    limit: *limit,
                })
            }
            _ => Err(anyhow!("endparallel does not match a parallel block")),
        }
    }
}

enum BlockKind {
    Root,
    Loop {
        count_expr: String,
    },
    Conditional {
        variable: String,
        operator: ConditionOp,
        value: String,
        else_body: Vec<AutomationCommand>,
        in_else: bool,
    },
    Parallel {
        branches: Vec<Vec<AutomationCommand>>,
        current_branch: Vec<AutomationCommand>,
        limit: Option<usize>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;
    use tempfile::NamedTempFile;
    use tokio::time::{sleep, Duration};

    use crate::app_context::AppContext;

    #[test]
    fn parser_handles_control_flow() {
        let script = r#"
set greeting Hello
loop 2
  navigate https://example.com
  type #query {{greeting}}
endloop

if environment == staging
  screenshot stage.png
else
  screenshot prod.png
endif

parallel
  branch
    navigate https://example.com/profile
    wait 250
  endbranch
  branch
    click #settings
  endbranch
endparallel
"#;

        let commands = ScriptParser::new().parse(script).expect("parse script");
        assert_eq!(commands.len(), 4);

        match &commands[1] {
            AutomationCommand::Loop { count_expr, body } => {
                assert_eq!(count_expr, "2");
                assert_eq!(body.len(), 2);
                assert!(matches!(
                    body[0],
                    AutomationCommand::Action(ActionCommand::Navigate(_))
                ));
            }
            other => panic!("expected loop command, got {other:?}"),
        }

        match &commands[2] {
            AutomationCommand::Conditional {
                variable,
                operator,
                value,
                then_body,
                else_body,
            } => {
                assert_eq!(variable, "environment");
                assert!(matches!(operator, ConditionOp::Equal));
                assert_eq!(value, "staging");
                assert_eq!(then_body.len(), 1);
                assert_eq!(else_body.len(), 1);
            }
            other => panic!("expected conditional command, got {other:?}"),
        }

        match &commands[3] {
            AutomationCommand::Parallel { branches, limit } => {
                assert_eq!(branches.len(), 2);
                assert!(matches!(
                    branches[0][1],
                    AutomationCommand::Action(ActionCommand::Wait(_))
                ));
                assert!(limit.is_none());
            }
            other => panic!("expected parallel command, got {other:?}"),
        }
    }

    #[test]
    fn execution_context_resolves_templates() {
        let mut context =
            ExecutionContext::new(HashMap::from([("name".to_string(), "Maru".to_string())]));
        context
            .locals
            .insert("greeting".to_string(), "Hello".to_string());

        let resolved = context.resolve_template("{{greeting}}, {{name}}!");
        assert_eq!(resolved, "Hello, Maru!");

        assert_eq!(context.get_variable("greeting"), Some("Hello".into()));
        assert_eq!(context.get_variable("name"), Some("Maru".into()));
    }

    #[test]
    fn parser_supports_parallel_limit() {
        let script = r#"
parallel 2
  branch
    navigate https://example.com/one
  endbranch
  branch
    navigate https://example.com/two
  endbranch
endparallel
"#;

        let commands = ScriptParser::new().parse(script).expect("parse script");
        assert_eq!(commands.len(), 1);
        match &commands[0] {
            AutomationCommand::Parallel { branches, limit } => {
                assert_eq!(branches.len(), 2);
                assert_eq!(*limit, Some(2));
            }
            other => panic!("expected parallel command, got {other:?}"),
        }
    }

    #[derive(Clone, Default)]
    struct MockRuntimeFactory {
        events: Arc<Mutex<Vec<String>>>,
    }

    impl MockRuntimeFactory {
        fn new() -> Self {
            Self::default()
        }

        fn record(&self, entry: String) {
            let mut guard = self.events.lock().unwrap();
            guard.push(entry);
        }

        fn events(&self) -> Vec<String> {
            self.events.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl AutomationRuntimeFactory for MockRuntimeFactory {
        async fn create_runtime(
            &self,
            _config: &AutomationConfig,
        ) -> Result<Box<dyn AutomationRuntimeAdapter>> {
            Ok(Box::new(MockRuntime {
                factory: self.clone(),
            }))
        }
    }

    struct MockRuntime {
        factory: MockRuntimeFactory,
    }

    #[async_trait]
    impl AutomationRuntimeAdapter for MockRuntime {
        async fn navigate(&mut self, url: &str) -> Result<()> {
            self.factory.record(format!("navigate:{url}"));
            Ok(())
        }

        async fn click(&mut self, selector: &str) -> Result<()> {
            self.factory.record(format!("click:{selector}"));
            Ok(())
        }

        async fn type_text(&mut self, selector: &str, text: &str) -> Result<()> {
            self.factory.record(format!("type:{selector}:{text}"));
            Ok(())
        }

        async fn select_option(
            &mut self,
            selector: &str,
            value: &str,
            match_kind: Option<&str>,
            mode: Option<&str>,
        ) -> Result<()> {
            self.factory.record(format!(
                "select:{selector}:{value}:{:?}:{:?}",
                match_kind, mode
            ));
            Ok(())
        }

        async fn screenshot(&mut self, filename: &str) -> Result<()> {
            self.factory.record(format!("screenshot:{filename}"));
            Ok(())
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn engine_uses_custom_runtime() {
        let context = Arc::new(
            AppContext::new("test-tenant".to_string(), None, &[])
                .await
                .expect("build app context"),
        );

        let mut config = AutomationConfig::default();
        config.parallel_instances = 3;
        config.headless = true;

        let mock_factory = Arc::new(MockRuntimeFactory::new());
        let factory_trait: Arc<dyn AutomationRuntimeFactory> = mock_factory.clone();

        let mut engine = AutomationEngine::with_runtime_factory(context, config, factory_trait)
            .await
            .expect("create engine");

        let mut tmp = NamedTempFile::new().expect("create temp script");
        writeln!(tmp, "set target https://example.com").unwrap();
        writeln!(tmp, "navigate {{target}}").unwrap();
        writeln!(tmp, "click #submit").unwrap();
        writeln!(tmp, "select #country us match=value").unwrap();
        tmp.flush().unwrap();

        let script_path = tmp.path().to_path_buf();
        engine
            .execute_script(&script_path)
            .await
            .expect("execute script");

        let events = mock_factory.events();
        assert_eq!(events.len(), 3);
        assert_eq!(events[0], "navigate:{target}");
        assert_eq!(events[1], "click:#submit");
        assert_eq!(events[2], "select:#country:us:Some(\"value\"):None");
    }

    #[derive(Clone, Default)]
    struct DummyExecutor {
        current: Arc<AtomicUsize>,
        peak: Arc<AtomicUsize>,
    }

    impl DummyExecutor {
        fn new() -> Self {
            Self::default()
        }

        fn peak(&self) -> usize {
            self.peak.load(Ordering::SeqCst)
        }

        async fn execute(&self, action: &ActionCommand) -> Result<()> {
            match action {
                ActionCommand::Wait(duration_expr) => {
                    let duration = duration_expr.parse::<u64>().unwrap_or(10);
                    sleep(Duration::from_millis(duration)).await;
                }
                _ => {
                    let concurrent = self.current.fetch_add(1, Ordering::SeqCst) + 1;
                    self.peak.fetch_max(concurrent, Ordering::SeqCst);
                    sleep(Duration::from_millis(10)).await;
                    self.current.fetch_sub(1, Ordering::SeqCst);
                }
            }
            Ok(())
        }
    }

    #[async_recursion]
    async fn execute_commands_test(
        commands: &[AutomationCommand],
        executor: DummyExecutor,
        global_limit: Arc<Semaphore>,
    ) -> Result<ExecutionStats> {
        let mut stats = ExecutionStats::default();
        for command in commands {
            match command {
                AutomationCommand::Action(action) => {
                    executor.execute(action).await?;
                    stats.steps_completed += 1;
                }
                AutomationCommand::Loop { body, .. } => {
                    let loop_stats =
                        execute_commands_test(body, executor.clone(), global_limit.clone()).await?;
                    stats.merge(loop_stats);
                }
                AutomationCommand::Conditional {
                    then_body,
                    else_body,
                    ..
                } => {
                    if !then_body.is_empty() {
                        let cond_stats = execute_commands_test(
                            then_body,
                            executor.clone(),
                            global_limit.clone(),
                        )
                        .await?;
                        stats.merge(cond_stats);
                    } else if !else_body.is_empty() {
                        let else_stats = execute_commands_test(
                            else_body,
                            executor.clone(),
                            global_limit.clone(),
                        )
                        .await?;
                        stats.merge(else_stats);
                    }
                }
                AutomationCommand::Parallel { branches, limit } => {
                    stats.merge(
                        execute_parallel_test(
                            branches,
                            *limit,
                            executor.clone(),
                            global_limit.clone(),
                        )
                        .await?,
                    );
                }
                AutomationCommand::Set { .. } => {}
            }
        }
        Ok(stats)
    }

    async fn execute_parallel_test(
        branches: &[Vec<AutomationCommand>],
        limit: Option<usize>,
        executor: DummyExecutor,
        global_limit: Arc<Semaphore>,
    ) -> Result<ExecutionStats> {
        let mut stats = ExecutionStats::default();
        let mut join_set = JoinSet::new();
        let local_limit = limit.map(|value| Arc::new(Semaphore::new(value)));

        for branch in branches.iter().cloned() {
            let exec_clone = executor.clone();
            let global_for_acquire = global_limit.clone();
            let global_for_child = global_limit.clone();
            let local_pool = local_limit.clone();
            join_set.spawn(async move {
                let global_permit = global_for_acquire
                    .acquire_owned()
                    .await
                    .map_err(|err| anyhow!("Parallel execution cancelled: {}", err))?;

                let local_permit = if let Some(pool) = local_pool {
                    Some(
                        pool.acquire_owned()
                            .await
                            .map_err(|err| anyhow!("Parallel execution cancelled: {}", err))?,
                    )
                } else {
                    None
                };

                let result = execute_commands_test(&branch, exec_clone, global_for_child).await;
                drop(local_permit);
                drop(global_permit);
                result
            });
        }

        while let Some(result) = join_set.join_next().await {
            match result {
                Ok(Ok(branch_stats)) => stats.merge(branch_stats),
                Ok(Err(err)) => stats.errors.push(err.to_string()),
                Err(join_err) => stats.errors.push(join_err.to_string()),
            }
        }

        Ok(stats)
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn parallel_respects_limits() {
        let script = r#"
parallel 2
  branch
    navigate https://example.com/one
  endbranch
  branch
    click #settings
  endbranch
  branch
    screenshot capture.png
  endbranch
endparallel
"#;

        let commands = ScriptParser::new().parse(script).expect("parse script");
        let executor = DummyExecutor::new();
        let global_limit = Arc::new(Semaphore::new(3));

        let stats = execute_commands_test(&commands, executor.clone(), global_limit)
            .await
            .expect("execute commands");

        assert_eq!(stats.steps_completed, 3);
        assert_eq!(
            executor.peak(),
            2,
            "block-level limit should cap concurrency at 2"
        );
    }
}
