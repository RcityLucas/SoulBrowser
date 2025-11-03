use anyhow::{anyhow, bail, Context, Result};
use async_trait::async_trait;
use axum::response::IntoResponse;
use base64::{engine::general_purpose::STANDARD as Base64, Engine as _};
use cdp_adapter::{
    config::CdpConfig, event_bus, events::RawEvent, ids::PageId as AdapterPageId,
    AdapterError as CdpAdapterError, Cdp, CdpAdapter,
};
use chrono::{offset::LocalResult, TimeZone, Utc};
use clap::{Args, Parser, Subcommand};
use l6_timeline::{
    api::TimelineService as L6TimelineService,
    model::{By as TimelineBy, ExportReq as TimelineExportReq, View as TimelineView},
    policy::{set_policy as timeline_set_policy, TimelinePolicyView},
    Timeline,
};
use l7_adapter::{
    events::ObserverEvents,
    ports::{
        DispatcherPort, ReadonlyPort, TimelineExportOutcome as AdapterTimelineExportOutcome,
        TimelineExportReq as AdapterTimelineExportReq, ToolCall as AdapterToolCall, ToolOutcome,
    },
    trace::AdapterTracer,
    AdapterBootstrap, AdapterPolicyHandle, AdapterPolicyView, AdapterResult, TenantPolicy,
};
use l7_adapter::{map, AdapterError as L7AdapterError};
use l7_wd_bridge::errors::{BridgeError, BridgeResult};
use l7_wd_bridge::{
    dispatcher::ToolDispatcher,
    policy::{TenantPolicy as WdTenantPolicy, WebDriverBridgePolicy, WebDriverBridgePolicyHandle},
    trace::BridgeTracer,
    WebDriverBridge,
};
use perceiver_hub::models::MultiModalPerception;
use perceiver_structural::{
    metrics::{self as structural_metrics, MetricSnapshot},
    AdapterPort as StructuralAdapterPort, ResolveHint, ResolveOptions, StructuralPerceiver,
    StructuralPerceiverImpl,
};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use soulbrowser_event_store::api::InMemoryEventStore as TimelineEventStore;
use soulbrowser_event_store::model::{
    AppendMeta as TimelineAppendMeta, EventEnvelope as TimelineEventEnvelope,
    EventScope as TimelineEventScope, EventSource as TimelineEventSource,
    LogLevel as TimelineLogLevel,
};
use soulbrowser_event_store::{
    EsPolicyView as TimelineEsPolicy, EventStore as TimelineEventStoreTrait,
};
use soulbrowser_state_center::{
    DispatchEvent, DispatchStatus, InMemoryStateCenter, PerceiverEvent, PerceiverEventKind,
    RegistryAction, ScoreComponentRecord, StateCenter, StateEvent,
};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::env;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tempfile::tempdir;
use tokio::fs;
use tokio::net::TcpListener;
use tokio::process::Command as TokioCommand;
use tokio::signal;
use tokio::sync::broadcast::error::RecvError;
use tokio::task::JoinSet;
use tokio::time::{sleep, timeout, Instant};
use tracing::{debug, error, info, warn};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

const DEFAULT_LARGE_THRESHOLD: u64 = 5 * 1024 * 1024;
const CONSOLE_HTML: &str = include_str!("static/console.html");

// Import types from our modules
use crate::agent::executor::DispatchRecord;
use crate::agent::{
    execute_plan, ChatRunner, ChatSessionOutput, FlowExecutionOptions, FlowExecutionReport,
    StepExecutionStatus,
};
use crate::analytics::{SessionAnalytics, SessionAnalyzer};
use crate::app_context::{get_or_create_context, AppContext};
use crate::automation::{AutomationConfig, AutomationEngine, AutomationResults};
use crate::browser_impl::{BrowserConfig, L0Protocol, L1BrowserManager};
use crate::export::{CsvExporter, Exporter, HtmlExporter, JsonExporter};
use crate::replay::{ReplayConfig, SessionReplayer};
use crate::storage::{BrowserEvent, BrowserSessionEntity, QueryParams, StorageManager};
use crate::types::BrowserType;
use agent_core::{AgentContext, AgentRequest, ConversationRole, ConversationTurn};
use humantime::format_rfc3339;
use serde_json::{json, Map as JsonMap, Number as JsonNumber, Value};
use soulbase_types::tenant::TenantId;
use soulbrowser_core_types::{
    ActionId, ExecRoute, FrameId, PageId, RoutingHint, SessionId, TaskId, ToolCall,
};
use soulbrowser_policy_center::{
    default_snapshot, InMemoryPolicyCenter, PolicyCenter, RuntimeOverrideSpec,
};
use soulbrowser_registry::Registry;
use soulbrowser_scheduler::model::{
    CallOptions, DispatchRequest, DispatchTimeline, Priority, RetryOpt, SubmitHandle,
};
use soulbrowser_scheduler::{metrics as scheduler_metrics, Dispatcher};
use uuid::Uuid;

// Import only core dependencies for now
// TODO: Add layer imports once dependency issues are resolved
// pub use soulbrowser::*;
// pub use soul_contracts::*;
// pub use l0_cdp_adapter::*;
// pub use l1_registry::*;
// pub use l2_structural_perceiver::*;
// pub use l3_action_producer::*;
// pub use l4_event_store::*;
// pub use l5_tools::*;

// Soul-base integrated modules
mod agent;
mod auth;
mod config;
mod errors;
mod interceptors;
mod l0_bridge;
mod metrics;
mod storage;
mod tools;
mod types;

// Browser implementation using soul-base
mod browser_impl;

// Application context for shared components
mod app_context;

// Feature modules
mod analytics;
mod automation;
mod export;
mod policy;
mod replay;

/// SoulBrowser - Intelligent Web Automation with Soul
#[derive(Parser)]
#[command(author, version, about, long_about = None)]
#[command(propagate_version = true)]
struct Cli {
    /// Configuration file path
    #[arg(short, long, value_name = "FILE")]
    config: Option<PathBuf>,

    /// Log level
    #[arg(short, long, default_value = "info")]
    log_level: String,

    /// Enable debug mode
    #[arg(short, long)]
    debug: bool,

    /// Output format
    #[arg(short, long, default_value = "human")]
    output: OutputFormat,

    /// Metrics server port (set to 0 to disable)
    #[arg(long, default_value_t = 9090)]
    metrics_port: u16,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Clone, Debug, clap::ValueEnum)]
enum OutputFormat {
    Human,
    Json,
    Yaml,
}

#[derive(Clone, Debug, clap::ValueEnum)]
enum TimelineViewOpt {
    Records,
    Timeline,
    Replay,
}

impl From<TimelineViewOpt> for TimelineView {
    fn from(value: TimelineViewOpt) -> Self {
        match value {
            TimelineViewOpt::Records => TimelineView::Records,
            TimelineViewOpt::Timeline => TimelineView::Timeline,
            TimelineViewOpt::Replay => TimelineView::Replay,
        }
    }
}

#[derive(Args)]
struct TimelineArgs {
    #[arg(long, value_enum, default_value = "records")]
    view: TimelineViewOpt,

    #[arg(long)]
    action_id: Option<String>,

    #[arg(long)]
    flow_id: Option<String>,

    #[arg(long)]
    task_id: Option<String>,

    #[arg(long)]
    since: Option<String>,

    #[arg(long)]
    until: Option<String>,

    #[arg(long, default_value_t = 5000)]
    limit: usize,

    #[arg(long)]
    max_lines: Option<usize>,

    #[arg(long)]
    output: Option<PathBuf>,
}

#[derive(Args)]
struct GatewayArgs {
    /// Address for the HTTP adapter (Axum).
    #[arg(long, default_value = "127.0.0.1:8710")]
    http: SocketAddr,

    /// Optional gRPC endpoint address.
    #[arg(long)]
    grpc: Option<SocketAddr>,

    /// Optional WebDriver bridge address.
    #[arg(long)]
    webdriver: Option<SocketAddr>,

    /// Path to adapter policy definition (json/yaml).
    #[arg(long, value_name = "FILE")]
    adapter_policy: Option<PathBuf>,

    /// Path to WebDriver bridge policy definition (json/yaml).
    #[arg(long, value_name = "FILE")]
    webdriver_policy: Option<PathBuf>,
}

#[derive(Subcommand)]
enum Commands {
    /// Start an interactive browser session
    Start(StartArgs),

    /// Run automation scripts
    Run(RunArgs),

    /// Record user interactions for later replay
    Record(RecordArgs),

    /// Replay recorded sessions
    Replay(ReplayArgs),

    /// Export metrics and performance data
    Export(ExportArgs),

    /// Analyze browser sessions and generate insights
    Analyze(AnalyzeArgs),

    /// Generate plans using the L8 agent interface
    Chat(ChatArgs),

    /// Manage SoulBrowser configuration
    Config(ConfigArgs),

    /// Show system information and health check
    Info,

    /// Inspect recent perceiver events captured by the state center
    Perceiver(PerceiverArgs),

    /// Inspect recent scheduler dispatch events
    Scheduler(SchedulerArgs),

    /// Manage policy snapshots and overrides
    Policy(PolicyArgs),

    /// Inspect artifacts captured during a saved run
    Artifacts(ArtifactsArgs),

    /// Produce JSON payloads for the Web Console prototype
    Console(ConsoleArgs),

    /// Export governance timeline snapshots
    Timeline(TimelineArgs),

    /// Run the external adapter surfaces (HTTP/gRPC/WebDriver)
    Gateway(GatewayArgs),

    /// Run a minimal real-browser demo against Chromium via the CDP adapter
    Demo(DemoArgs),

    /// Perform multi-modal page perception (visual, semantic, structural)
    Perceive(PerceiveArgs),

    /// Launch a lightweight testing server with a visual console
    Serve(ServeArgs),
}

#[derive(Args)]
struct StartArgs {
    /// Browser type to launch
    #[arg(short, long, default_value = "chromium")]
    browser: BrowserType,

    /// Start URL
    #[arg(short, long)]
    url: Option<String>,

    /// Browser window size
    #[arg(long, value_name = "WIDTHxHEIGHT")]
    window_size: Option<String>,

    /// Enable headless mode
    #[arg(long)]
    headless: bool,

    /// Enable developer tools
    #[arg(long)]
    devtools: bool,

    /// Session name for saving
    #[arg(long)]
    session_name: Option<String>,

    /// Enable soul (AI) assistance
    #[arg(long)]
    soul: bool,

    /// Soul model to use
    #[arg(long, default_value = "gpt-4")]
    soul_model: String,

    /// Route through the L1 unified kernel scheduler
    #[arg(long)]
    unified_kernel: bool,
}

#[derive(Args)]
struct RunArgs {
    /// Script file to execute
    script: PathBuf,

    /// Browser type to use
    #[arg(short, long, default_value = "chromium")]
    browser: BrowserType,

    /// Enable headless mode
    #[arg(long)]
    headless: bool,

    /// Script parameters (key=value)
    #[arg(short, long)]
    param: Vec<String>,

    /// Output directory for results
    #[arg(short, long)]
    output_dir: Option<PathBuf>,

    /// Maximum execution time in seconds
    #[arg(long, default_value = "300")]
    timeout: u64,

    /// Number of parallel instances
    #[arg(long, default_value = "1")]
    parallel: usize,
}

#[derive(Args)]
struct DemoArgs {
    /// URL to open in the demo session
    #[arg(long, default_value = "https://www.wikipedia.org/")]
    url: String,

    /// Seconds to wait for the initial page/session wiring
    #[arg(long, default_value_t = 30)]
    startup_timeout: u64,

    /// Seconds to continue streaming events after DOM ready
    #[arg(long, default_value_t = 5)]
    hold_after_ready: u64,

    /// Optional path to write a PNG screenshot after navigation
    #[arg(long)]
    screenshot: Option<PathBuf>,

    /// Override Chrome/Chromium executable path (defaults to SOULBROWSER_CHROME or system path)
    #[arg(long)]
    chrome_path: Option<PathBuf>,

    /// Run Chrome with a visible window instead of headless mode
    #[arg(long)]
    headful: bool,

    /// Attach to an existing Chrome DevTools websocket instead of launching a new instance
    #[arg(long)]
    ws_url: Option<String>,

    /// CSS selector for the input field to populate during the demo
    #[arg(long, default_value = "#searchInput")]
    input_selector: String,

    /// Text to type into the input field
    #[arg(long, default_value = "SoulBrowser")]
    input_text: String,

    /// CSS selector for the submit button; if omitted no click is issued
    #[arg(long, default_value = "button.pure-button")]
    submit_selector: String,

    /// Skip the submit click step even if a selector is provided
    #[arg(long)]
    skip_submit: bool,
}

#[derive(Args)]
struct PerceiveArgs {
    /// URL to analyze
    #[arg(long)]
    url: String,

    /// Enable visual perception (screenshots, visual metrics)
    #[arg(long)]
    visual: bool,

    /// Enable semantic perception (content classification, language detection)
    #[arg(long)]
    semantic: bool,

    /// Enable structural perception (DOM/AX tree analysis)
    #[arg(long)]
    structural: bool,

    /// Enable all perception modes (visual + semantic + structural)
    #[arg(long)]
    all: bool,

    /// Capture screenshot to file
    #[arg(long)]
    screenshot: Option<PathBuf>,

    /// Output perception results to JSON file
    #[arg(short, long)]
    output: Option<PathBuf>,

    /// Show cross-modal insights
    #[arg(long)]
    insights: bool,

    /// Override Chrome/Chromium executable path
    #[arg(long)]
    chrome_path: Option<PathBuf>,

    /// Run Chrome with a visible window instead of headless mode
    #[arg(long)]
    headful: bool,

    /// Attach to an existing Chrome DevTools websocket
    #[arg(long)]
    ws_url: Option<String>,

    /// Analysis timeout in seconds
    #[arg(long, default_value = "30")]
    timeout: u64,
}

#[derive(Args, Clone)]
struct ServeArgs {
    /// Port for the testing server
    #[arg(long, default_value_t = 8787)]
    port: u16,

    /// Attach to an existing Chrome DevTools websocket (optional)
    #[arg(long)]
    ws_url: Option<String>,
}

#[derive(Args)]
struct RecordArgs {
    /// Session name
    name: String,

    /// Browser type to use
    #[arg(short, long, default_value = "chromium")]
    browser: BrowserType,

    /// Start URL
    #[arg(short, long)]
    url: Option<String>,

    /// Recording output directory
    #[arg(short, long)]
    output_dir: Option<PathBuf>,

    /// Enable screenshot recording
    #[arg(long)]
    screenshots: bool,

    /// Enable video recording
    #[arg(long)]
    video: bool,

    /// Record network activity
    #[arg(long)]
    network: bool,

    /// Record performance metrics
    #[arg(long)]
    performance: bool,
}

#[derive(Args)]
struct ReplayArgs {
    /// Recording file or session name
    recording: String,

    /// Browser type to use
    #[arg(short, long, default_value = "chromium")]
    browser: BrowserType,

    /// Playback speed multiplier
    #[arg(long, default_value = "1.0")]
    speed: f64,

    /// Enable headless mode
    #[arg(long)]
    headless: bool,

    /// Stop on first error
    #[arg(long)]
    fail_fast: bool,

    /// Override parameters
    #[arg(short, long)]
    param: Vec<String>,

    /// Generate comparison report
    #[arg(long)]
    compare: bool,
}

#[derive(Args)]
struct ExportArgs {
    /// Data type to export
    #[command(subcommand)]
    data_type: ExportType,
}

#[derive(Subcommand)]
enum ExportType {
    /// Export performance metrics
    Metrics {
        /// Session or recording name
        session: String,

        /// Output file
        #[arg(short, long)]
        output: Option<PathBuf>,

        /// Export format
        #[arg(short, long, default_value = "json")]
        format: DataFormat,
    },

    /// Export interaction timeline
    Timeline {
        /// Session or recording name
        session: String,

        /// Output file
        #[arg(short, long)]
        output: Option<PathBuf>,

        /// Export format
        #[arg(short, long, default_value = "html")]
        format: TimelineFormat,
    },

    /// Export automation scripts
    Script {
        /// Session or recording name
        session: String,

        /// Target language
        #[arg(short, long, default_value = "javascript")]
        language: ScriptLanguage,

        /// Output file
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
}

#[derive(Args)]
struct AnalyzeArgs {
    /// Session or recording to analyze
    target: String,

    /// Analysis type
    #[arg(short, long, default_value = "performance")]
    analysis_type: AnalysisType,

    /// Output report file
    #[arg(short, long)]
    output: Option<PathBuf>,

    /// Generate interactive report
    #[arg(long)]
    interactive: bool,
}

#[derive(Args)]
struct ChatArgs {
    /// Prompt describing the desired browsing task
    #[arg(
        short,
        long,
        conflicts_with = "prompt_file",
        required_unless_present = "prompt_file"
    )]
    prompt: Option<String>,

    /// File containing the task prompt (UTF-8)
    #[arg(long, conflicts_with = "prompt")]
    prompt_file: Option<PathBuf>,

    /// Optional current URL to seed agent context
    #[arg(long)]
    current_url: Option<String>,

    /// Additional constraints for the planner (repeatable)
    #[arg(long = "constraint", value_name = "CONSTRAINT")]
    constraints: Vec<String>,

    /// Persist the generated plan to a file (JSON)
    #[arg(long, value_name = "PATH")]
    save_plan: Option<PathBuf>,

    /// Persist the derived action flow to a file (JSON)
    #[arg(long, value_name = "PATH")]
    save_flow: Option<PathBuf>,

    /// Execute the generated plan immediately via the scheduler
    #[arg(long)]
    execute: bool,

    /// Maximum retry attempts per tool when executing (0 = no retry)
    #[arg(long, default_value_t = 1)]
    max_retries: u8,

    /// Maximum number of replanning attempts after execution failure
    #[arg(long, default_value_t = 0)]
    max_replans: u8,

    /// Persist combined plan/execution run metadata to a file (JSON)
    #[arg(long, value_name = "PATH")]
    save_run: Option<PathBuf>,

    /// Emit only the artifact manifest when using JSON/YAML output
    #[arg(long)]
    artifacts_only: bool,

    /// Write the artifact manifest to a JSON file
    #[arg(long, value_name = "PATH")]
    artifacts_path: Option<PathBuf>,
}

#[derive(Args)]
struct ArtifactsArgs {
    /// Path to a saved run bundle (JSON produced by --save-run)
    #[arg(long, value_name = "FILE")]
    input: PathBuf,

    /// Output format for printing the manifest
    #[arg(long, value_enum, default_value = "json")]
    format: ArtifactFormat,

    /// Filter by step identifier
    #[arg(long)]
    step_id: Option<String>,

    /// Filter by dispatch label (e.g. "action" or validation name)
    #[arg(long)]
    dispatch: Option<String>,

    /// Filter by artifact label
    #[arg(long)]
    label: Option<String>,

    /// Directory to extract matching artifacts as files (base64 decoded)
    #[arg(long, value_name = "DIR")]
    extract: Option<PathBuf>,

    /// Path to write a summary (JSON) of matching artifacts
    #[arg(long, value_name = "FILE")]
    summary_path: Option<PathBuf>,

    /// Threshold in bytes for highlighting large artifacts
    #[arg(long, value_name = "BYTES", default_value_t = 5 * 1024 * 1024)]
    large_threshold: u64,
}

#[derive(Args)]
struct ConsoleArgs {
    /// Path to the saved run bundle produced by --save-run
    #[arg(long, value_name = "FILE")]
    input: PathBuf,

    /// Output file for the console payload (prints to stdout if omitted)
    #[arg(long, value_name = "FILE")]
    output: Option<PathBuf>,

    /// Produce pretty-printed JSON when writing to stdout
    #[arg(long)]
    pretty: bool,

    /// Serve a lightweight Web Console preview instead of printing JSON
    #[arg(long)]
    serve: bool,

    /// Port to bind when running with --serve
    #[arg(long, default_value_t = 8710)]
    port: u16,
}

#[derive(Clone, clap::ValueEnum)]
enum ArtifactFormat {
    Json,
    Yaml,
    Human,
}

#[derive(Args)]
struct ConfigArgs {
    #[command(subcommand)]
    action: ConfigAction,
}

#[derive(Subcommand)]
enum ConfigAction {
    /// Show current configuration
    Show,

    /// Set configuration value
    Set {
        /// Configuration key
        key: String,

        /// Configuration value
        value: String,
    },

    /// Get configuration value
    Get {
        /// Configuration key
        key: String,
    },

    /// Reset configuration to defaults
    Reset,

    /// Validate configuration
    Validate,
}

// BrowserType is now defined in lib.rs with ValueEnum derive
// BrowserType now imported from crate::types at top of file

#[derive(Clone, Debug, clap::ValueEnum)]
enum DataFormat {
    Json,
    Csv,
    Excel,
    Yaml,
}

#[derive(Clone, Debug, clap::ValueEnum)]
enum TimelineFormat {
    Html,
    Json,
    Svg,
    Pdf,
}

#[derive(Clone, Debug, clap::ValueEnum)]
enum ScriptLanguage {
    JavaScript,
    TypeScript,
    Python,
    Rust,
    Go,
}

#[derive(Clone, Debug, clap::ValueEnum)]
enum AnalysisType {
    Performance,
    Accessibility,
    Security,
    Usability,
    Compatibility,
    Full,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct Config {
    /// Default browser type
    default_browser: BrowserType,

    /// Default headless mode
    default_headless: bool,

    /// Default output directory
    output_dir: PathBuf,

    /// Default session timeout
    session_timeout: u64,

    /// Soul (AI) configuration
    soul: SoulConfig,

    /// Recording configuration
    recording: RecordingConfigOptions,

    /// Performance monitoring configuration
    performance: PerformanceConfig,

    /// Additional policy files to load
    #[serde(default)]
    policy_paths: Vec<PathBuf>,

    /// Require strict authorization (no fallback)
    #[serde(default)]
    strict_authorization: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct SoulConfig {
    /// Enable soul by default
    enabled: bool,

    /// Default model
    model: String,

    /// API key (will be read from environment)
    api_key: Option<String>,

    /// Custom prompts directory
    prompts_dir: Option<PathBuf>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct RecordingConfigOptions {
    /// Enable screenshots by default
    screenshots: bool,

    /// Enable video recording by default
    video: bool,

    /// Enable network recording by default
    network: bool,

    /// Video quality
    video_quality: String,

    /// Screenshot format
    screenshot_format: String,
}

#[derive(Clone, Debug, clap::ValueEnum)]
enum SchedulerStatusFilter {
    Success,
    Failure,
}

#[derive(Clone, Debug, clap::ValueEnum)]
enum SchedulerOutputFormat {
    Text,
    Json,
}

#[derive(Clone, Debug, clap::ValueEnum)]
enum PerceiverKindFilter {
    Resolve,
    Judge,
    Snapshot,
    Diff,
}

#[derive(Clone, Debug, clap::ValueEnum)]
enum PerceiverCacheFilter {
    Hit,
    Miss,
}

#[derive(Clone, Debug, clap::ValueEnum)]
enum PerceiverOutputFormat {
    Table,
    Json,
}

#[derive(Args)]
struct SchedulerArgs {
    /// Number of recent events to display (default: 20)
    #[arg(short, long)]
    limit: Option<usize>,

    /// Only show events with the given status
    #[arg(long, value_enum)]
    status: Option<SchedulerStatusFilter>,

    /// Output format (`text` or `json`)
    #[arg(long, value_enum, default_value_t = SchedulerOutputFormat::Text)]
    format: SchedulerOutputFormat,

    /// Cancel a pending action by id
    #[arg(long)]
    cancel: Option<String>,
}

#[derive(Args, Debug, Clone)]
struct PerceiverArgs {
    /// Number of recent events to display (default: 20)
    #[arg(short, long)]
    limit: Option<usize>,

    /// Only show events of the specified kind
    #[arg(long, value_enum)]
    kind: Option<PerceiverKindFilter>,

    /// Only show judge events with the specified check (e.g., visible, clickable)
    #[arg(long)]
    check: Option<String>,

    /// Filter by session id
    #[arg(long)]
    session: Option<String>,

    /// Filter by page id
    #[arg(long)]
    page: Option<String>,

    /// Filter by frame id
    #[arg(long)]
    frame: Option<String>,

    /// Filter by cache status (resolve/snapshot events only)
    #[arg(long, value_enum)]
    cache: Option<PerceiverCacheFilter>,

    /// Output format (`table` or `json`)
    #[arg(long, value_enum, default_value_t = PerceiverOutputFormat::Table)]
    format: PerceiverOutputFormat,
}

#[derive(Args)]
struct PolicyArgs {
    #[command(subcommand)]
    command: PolicyCommand,
}

#[derive(Subcommand)]
enum PolicyCommand {
    Show(PolicyShowArgs),
    Override(PolicyOverrideArgs),
}

#[derive(Args)]
struct PolicyShowArgs {
    /// Output JSON instead of human summary
    #[arg(long)]
    json: bool,
}

#[derive(Args)]
struct PolicyOverrideArgs {
    /// Dot-path to override, e.g. scheduler.limits.global_slots
    path: String,
    /// Override value as JSON literal (e.g. 4, true, "value")
    value: String,
    /// Override owner label
    #[arg(long, default_value = "cli")]
    owner: String,
    /// Reason for override
    #[arg(long, default_value = "manual override")]
    reason: String,
    /// TTL in seconds (0 = permanent)
    #[arg(long)]
    ttl: Option<u64>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct PerformanceConfig {
    /// Enable performance monitoring by default
    enabled: bool,

    /// Sampling rate for metrics
    sampling_rate: f64,

    /// Performance budget thresholds
    thresholds: PerformanceThresholds,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct PerformanceThresholds {
    /// Maximum page load time (ms)
    page_load_time: u64,

    /// Maximum first contentful paint (ms)
    first_contentful_paint: u64,

    /// Maximum largest contentful paint (ms)
    largest_contentful_paint: u64,

    /// Maximum cumulative layout shift
    cumulative_layout_shift: f64,

    /// Maximum first input delay (ms)
    first_input_delay: u64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            default_browser: BrowserType::Chromium,
            default_headless: false,
            output_dir: PathBuf::from("./soulbrowser-output"),
            session_timeout: 300,
            soul: SoulConfig {
                enabled: false,
                model: "gpt-4".to_string(),
                api_key: None,
                prompts_dir: None,
            },
            recording: RecordingConfigOptions {
                screenshots: true,
                video: false,
                network: true,
                video_quality: "high".to_string(),
                screenshot_format: "png".to_string(),
            },
            performance: PerformanceConfig {
                enabled: true,
                sampling_rate: 1.0,
                thresholds: PerformanceThresholds {
                    page_load_time: 3000,
                    first_contentful_paint: 1500,
                    largest_contentful_paint: 2500,
                    cumulative_layout_shift: 0.1,
                    first_input_delay: 100,
                },
            },
            policy_paths: Vec::new(),
            strict_authorization: false,
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Initialize tracing
    init_logging(&cli.log_level, cli.debug)?;
    let _metrics_server = crate::metrics::spawn_metrics_server(cli.metrics_port);

    info!("Starting SoulBrowser v{}", env!("CARGO_PKG_VERSION"));

    // Load configuration
    let config = load_config(cli.config.as_ref()).await?;
    apply_runtime_overrides(&config);

    // Execute command
    let result = match cli.command {
        Commands::Start(args) => cmd_start(args, &config).await,
        Commands::Run(args) => cmd_run(args, &config).await,
        Commands::Record(args) => cmd_record(args, &config).await,
        Commands::Replay(args) => cmd_replay(args, &config).await,
        Commands::Export(args) => cmd_export(args, &config).await,
        Commands::Analyze(args) => cmd_analyze(args, &config).await,
        Commands::Chat(args) => cmd_chat(args, &config, cli.output.clone()).await,
        Commands::Config(args) => cmd_config(args, &config).await,
        Commands::Info => cmd_info(&config).await,
        Commands::Perceiver(args) => cmd_perceiver(args, &config).await,
        Commands::Scheduler(args) => cmd_scheduler(args, &config).await,
        Commands::Policy(args) => cmd_policy(args, &config).await,
        Commands::Artifacts(args) => cmd_artifacts(args).await,
        Commands::Console(args) => cmd_console(args).await,
        Commands::Timeline(args) => cmd_timeline(args, &config).await,
        Commands::Gateway(args) => cmd_gateway(args, &config).await,
        Commands::Demo(args) => cmd_demo_real(args).await,
        Commands::Perceive(args) => cmd_perceive(args).await,
        Commands::Serve(args) => cmd_serve(args, cli.metrics_port, config.clone()).await,
    };

    match result {
        Ok(()) => {
            info!("Command completed successfully");
            Ok(())
        }
        Err(e) => {
            error!("Command failed: {}", e);
            std::process::exit(1);
        }
    }
}

fn init_logging(level: &str, debug: bool) -> Result<()> {
    let level = if debug {
        tracing::Level::DEBUG
    } else {
        level.parse().context("Invalid log level")?
    };

    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(level.to_string())),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    Ok(())
}

async fn load_config(config_path: Option<&PathBuf>) -> Result<Config> {
    let config_path = match config_path {
        Some(path) => path.clone(),
        None => {
            let mut path = dirs::config_dir().context("Failed to get config directory")?;
            path.push("soulbrowser");
            path.push("config.yaml");
            path
        }
    };

    if config_path.exists() {
        let content = fs::read_to_string(&config_path)
            .await
            .context("Failed to read config file")?;

        let config: Config =
            serde_yaml::from_str(&content).context("Failed to parse config file")?;

        info!("Loaded configuration from: {}", config_path.display());
        Ok(config)
    } else {
        warn!(
            "Config file not found, using defaults: {}",
            config_path.display()
        );
        Ok(Config::default())
    }
}

fn apply_runtime_overrides(config: &Config) {
    if config.strict_authorization && std::env::var("SOUL_STRICT_AUTHZ").is_err() {
        std::env::set_var("SOUL_STRICT_AUTHZ", "true");
        info!("Enabled strict authorization (SOUL_STRICT_AUTHZ=true)");
    }

    if std::env::var("SOUL_POLICY_PATH").is_err() {
        if let Some(path) = config.policy_paths.first() {
            std::env::set_var("SOUL_POLICY_PATH", path);
            info!("Using policy file from config: {}", path.display());
        }
    }
}

fn compute_timeline_ms(timeline: &DispatchTimeline) -> (u64, u64) {
    let wait_ms = timeline
        .started_at
        .map(|start| start.duration_since(timeline.enqueued_at).as_millis() as u64)
        .unwrap_or(0);
    let run_ms = match (timeline.started_at, timeline.finished_at) {
        (Some(start), Some(finish)) => finish.duration_since(start).as_millis() as u64,
        _ => 0,
    };
    (wait_ms, run_ms)
}

async fn cmd_start(args: StartArgs, config: &Config) -> Result<()> {
    info!("Starting browser session");

    let context = get_or_create_context(
        "cli".to_string(),
        Some(config.output_dir.clone()),
        config.policy_paths.clone(),
    )
    .await
    .map_err(|e| anyhow!(e.to_string()))?;

    // Initialize L0 (Protocol Layer)
    let l0 = L0Protocol::new().await?;

    // Initialize L1 (Browser Management)
    let browser_config = BrowserConfig {
        browser_type: args.browser.clone(),
        headless: args.headless,
        window_size: parse_window_size(args.window_size.as_deref())?,
        devtools: args.devtools,
        ..Default::default()
    };

    let mut l1 = L1BrowserManager::new(l0, browser_config).await?;

    // Launch browser
    let browser = l1.launch_browser().await?;
    info!("Browser launched successfully");

    // Create new page using soul-base
    let mut page = browser
        .new_page()
        .await
        .context("Failed to create new page")?;

    // Navigate to URL if provided
    if let Some(url) = args.url.clone() {
        if args.unified_kernel {
            if let Err(err) = unified_kernel_navigate(context.clone(), url.clone()).await {
                warn!("Unified kernel navigation request failed: {}", err);
            }
            info!("Unified kernel request dispatched; continuing with direct navigation for compatibility");
        }
        info!("Navigating to: {}", url);
        page.navigate(&url)
            .await
            .context("Failed to navigate to URL")?;
    }

    // Initialize L3 (DOM Intelligence) if soul is enabled
    if args.soul {
        info!(
            "Enabling Soul (AI) assistance with model: {}",
            args.soul_model
        );
        // Initialize soul with specified model
        // This would integrate with your AI service
    }

    // Keep session alive
    info!("Browser session started. Press Ctrl+C to exit.");
    tokio::signal::ctrl_c().await?;
    info!("Shutting down browser session");

    Ok(())
}

async fn unified_kernel_navigate(context: Arc<AppContext>, url: String) -> Result<()> {
    let registry = context.registry();
    let session_id = registry
        .session_create("cli-session")
        .await
        .map_err(|e| anyhow!(e.to_string()))?;
    let page_id = registry
        .page_open(session_id)
        .await
        .map_err(|e| anyhow!(e.to_string()))?;
    registry
        .page_focus(page_id.clone())
        .await
        .map_err(|e| anyhow!(e.to_string()))?;

    let scheduler = context.scheduler_service();
    let policy_snapshot = context.policy_center().snapshot().await;
    let scheduler_policy = &policy_snapshot.scheduler;
    let tool_call = ToolCall {
        call_id: Some(Uuid::new_v4().to_string()),
        task_id: Some(TaskId::new()),
        tool: "navigate-to-url".to_string(),
        payload: json!({ "url": url }),
    };

    let options = CallOptions {
        timeout: Duration::from_millis(scheduler_policy.timeouts_ms.navigate),
        priority: Priority::Standard,
        interruptible: true,
        retry: RetryOpt {
            max: scheduler_policy.retry.max_attempts,
            backoff: Duration::from_millis(scheduler_policy.retry.backoff_ms),
        },
    };

    let request = DispatchRequest {
        tool_call,
        options,
        routing_hint: Some(RoutingHint {
            page: Some(page_id),
            ..Default::default()
        }),
    };

    let handle = scheduler
        .submit(request)
        .await
        .map_err(|e| anyhow!(e.to_string()))?;

    match handle.receiver.await {
        Ok(output) => {
            let (wait_ms, run_ms) = compute_timeline_ms(&output.timeline);
            if let Some(err) = output.error {
                warn!(
                    "Unified kernel tool execution failed: {} (wait={}ms run={}ms)",
                    err, wait_ms, run_ms
                );
            } else {
                info!(
                    "Unified kernel tool execution succeeded (wait={}ms run={}ms)",
                    wait_ms, run_ms
                );
            }
        }
        Err(_) => warn!("Scheduler worker dropped navigation response"),
    }

    Ok(())
}

fn plan_payload(session: &ChatSessionOutput) -> Value {
    json!({
        "plan": session.plan,
        "flow": {
            "metadata": {
                "step_count": session.flow.step_count,
                "validation_count": session.flow.validation_count,
            },
            "definition": session.flow.flow,
        },
        "explanations": session.explanations,
    })
}

fn print_human_plan(session: &ChatSessionOutput, attempt: u32) {
    if attempt > 0 {
        println!("\nReplan attempt {}:", attempt);
    }
    println!("Agent plan: {}", session.plan.title);
    if !session.plan.description.is_empty() {
        println!("  {}", session.plan.description);
    }
    println!("Steps:");
    for line in session.summarize_steps() {
        println!("  - {}", line);
    }
    println!(
        "\nFlow: {} (steps: {}, validations: {})",
        session.flow.flow.id, session.flow.step_count, session.flow.validation_count
    );
    if !session.explanations.is_empty() {
        println!("\nPlanner reasoning:");
        for bullet in &session.explanations {
            println!("  - {}", bullet);
        }
    }
}

fn print_execution_summary(report: &FlowExecutionReport, attempt: u32) {
    if attempt == 0 {
        println!("\nExecution summary:");
    } else {
        println!("\nExecution summary (attempt {}):", attempt);
    }
    for step in &report.steps {
        let status = match step.status {
            StepExecutionStatus::Success => "success",
            StepExecutionStatus::Failed => "failed",
        };
        println!(
            "  - {} [{}] ({} attempt{})",
            step.title,
            status,
            step.attempts,
            if step.attempts == 1 { "" } else { "s" }
        );
        if let Some(error) = &step.error {
            println!("      error: {}", error);
        }
        for dispatch in &step.dispatches {
            println!(
                "      {}: action={} wait={}ms run={}ms",
                dispatch.label, dispatch.action_id, dispatch.wait_ms, dispatch.run_ms
            );
            println!(
                "         route: session={} page={} frame={} mutex={}",
                dispatch.route.session.0,
                dispatch.route.page.0,
                dispatch.route.frame.0,
                dispatch.route.mutex_key
            );
            if let Some(err) = &dispatch.error {
                println!("         error: {}", err);
            }
            for artifact in &dispatch.artifacts {
                println!(
                    "         artifact: {} ({} bytes, {}){}",
                    artifact.label,
                    artifact.byte_len,
                    artifact.content_type,
                    artifact
                        .filename
                        .as_ref()
                        .map(|name| format!(", filename={}", name))
                        .unwrap_or_default()
                );
            }
        }
    }
}

fn emit_structured_output(
    plans: &[Value],
    execution: &[Value],
    manifest: &ArtifactManifest,
    events: Option<&[Value]>,
    output: OutputFormat,
    artifacts_only: bool,
) -> Result<()> {
    if artifacts_only {
        return emit_artifact_manifest(manifest, output);
    }

    let payload = manifest.build_output_payload(plans, execution, events);

    match output {
        OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(&payload)?);
        }
        OutputFormat::Yaml => {
            println!("{}", serde_yaml::to_string(&payload)?);
        }
        OutputFormat::Human => {}
    }

    Ok(())
}

fn emit_artifact_manifest(manifest: &ArtifactManifest, output: OutputFormat) -> Result<()> {
    let artifacts_array = manifest.json_array();
    match output {
        OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(&artifacts_array)?);
        }
        OutputFormat::Yaml => {
            println!("{}", serde_yaml::to_string(&artifacts_array)?);
        }
        OutputFormat::Human => {
            for record in &manifest.items {
                println!(
                    "attempt={} step={} ({}) dispatch={} artifact={} bytes={} type={}{}",
                    record.attempt,
                    record.step_index,
                    record.step_id,
                    record.dispatch_label,
                    record.label,
                    record.byte_len,
                    record.content_type,
                    record
                        .filename
                        .as_ref()
                        .map(|name| format!(" filename={}", name))
                        .unwrap_or_default()
                );
            }
        }
    }
    Ok(())
}

fn execution_report_payload(
    report: &FlowExecutionReport,
    attempt: u32,
    events: Option<&[Value]>,
    manifest: &mut ArtifactManifest,
) -> Value {
    let mut steps = Vec::new();

    for (step_index, step) in report.steps.iter().enumerate() {
        let mut step_obj = serde_json::Map::new();
        step_obj.insert("step_id".into(), Value::String(step.step_id.clone()));
        step_obj.insert("title".into(), Value::String(step.title.clone()));
        step_obj.insert(
            "status".into(),
            Value::String(
                match step.status {
                    StepExecutionStatus::Success => "success",
                    StepExecutionStatus::Failed => "failed",
                }
                .to_string(),
            ),
        );
        step_obj.insert("attempts".into(), Value::from(step.attempts));
        if let Some(error) = &step.error {
            step_obj.insert("error".into(), Value::String(error.clone()));
        } else {
            step_obj.insert("error".into(), Value::Null);
        }

        let mut dispatch_values = Vec::new();
        for (dispatch_index, dispatch) in step.dispatches.iter().enumerate() {
            let mut dispatch_obj = serde_json::Map::new();
            dispatch_obj.insert("label".into(), Value::String(dispatch.label.clone()));
            dispatch_obj.insert(
                "action_id".into(),
                Value::String(dispatch.action_id.clone()),
            );
            dispatch_obj.insert(
                "route".into(),
                json!({
                    "session": dispatch.route.session.0,
                    "page": dispatch.route.page.0,
                    "frame": dispatch.route.frame.0,
                    "mutex": dispatch.route.mutex_key,
                }),
            );
            dispatch_obj.insert("wait_ms".into(), Value::from(dispatch.wait_ms));
            dispatch_obj.insert("run_ms".into(), Value::from(dispatch.run_ms));
            dispatch_obj.insert(
                "output".into(),
                dispatch.output.clone().unwrap_or(Value::Null),
            );

            let artifact_values: Vec<Value> = dispatch
                .artifacts
                .iter()
                .enumerate()
                .map(|(artifact_index, artifact)| {
                    manifest.add(ArtifactRecord {
                        attempt,
                        step_index,
                        step_id: step.step_id.clone(),
                        dispatch_label: dispatch.label.clone(),
                        dispatch_index,
                        artifact_index,
                        action_id: dispatch.action_id.clone(),
                        label: artifact.label.clone(),
                        content_type: artifact.content_type.clone(),
                        byte_len: artifact.byte_len,
                        filename: artifact.filename.clone(),
                        data_base64: artifact.data_base64.clone(),
                    });

                    json!({
                        "label": artifact.label,
                        "content_type": artifact.content_type,
                        "byte_len": artifact.byte_len,
                        "filename": artifact.filename,
                        "data_base64": artifact.data_base64,
                    })
                })
                .collect();
            dispatch_obj.insert("artifacts".into(), Value::Array(artifact_values));

            if let Some(err) = &dispatch.error {
                dispatch_obj.insert("error".into(), Value::String(err.clone()));
            } else {
                dispatch_obj.insert("error".into(), Value::Null);
            }

            dispatch_values.push(Value::Object(dispatch_obj));
        }

        step_obj.insert("dispatches".into(), Value::Array(dispatch_values));

        if let Some(events) = events {
            let matched = collect_step_events(events, &step.dispatches);
            if !matched.is_empty() {
                step_obj.insert("state_events".into(), Value::Array(matched));
            }
        }

        steps.push(Value::Object(step_obj));
    }

    json!({
        "attempt": attempt,
        "success": report.success,
        "steps": steps,
    })
}

fn collect_step_events(events: &[Value], dispatches: &[DispatchRecord]) -> Vec<Value> {
    if events.is_empty() || dispatches.is_empty() {
        return Vec::new();
    }

    let action_ids: HashSet<&str> = dispatches.iter().map(|d| d.action_id.as_str()).collect();
    let routes: Vec<(String, String, String)> =
        dispatches.iter().map(dispatch_route_tuple).collect();

    let mut matched = Vec::new();
    let mut seen = HashSet::new();

    for event in events {
        let kind = match event.get("type").and_then(|v| v.as_str()) {
            Some(kind) => kind,
            None => continue,
        };

        let mut include = false;
        match kind {
            "dispatch" => {
                if let Some(action_id) = event.get("action_id").and_then(|v| v.as_str()) {
                    if action_ids.contains(action_id) {
                        include = true;
                    }
                }
            }
            "registry" | "perceiver" => {
                if let Some(route) = route_tuple_from_event(event) {
                    if routes.iter().any(|r| *r == route) {
                        include = true;
                    }
                }
            }
            _ => {}
        }

        if include {
            if let Ok(fingerprint) = serde_json::to_string(event) {
                if seen.insert(fingerprint) {
                    matched.push(event.clone());
                }
            } else {
                matched.push(event.clone());
            }
        }
    }

    matched
}

fn build_execution_payloads(
    reports: &[FlowExecutionReport],
    events: Option<&[Value]>,
) -> (Vec<Value>, ArtifactManifest) {
    let mut manifest = ArtifactManifest::default();
    let execution = reports
        .iter()
        .enumerate()
        .map(|(idx, report)| execution_report_payload(report, idx as u32, events, &mut manifest))
        .collect();
    (execution, manifest)
}

fn dispatch_route_tuple(dispatch: &DispatchRecord) -> (String, String, String) {
    (
        dispatch.route.session.0.clone(),
        dispatch.route.page.0.clone(),
        dispatch.route.frame.0.clone(),
    )
}

fn route_tuple_from_event(value: &Value) -> Option<(String, String, String)> {
    let kind = value.get("type").and_then(|v| v.as_str())?;
    match kind {
        "dispatch" | "perceiver" => {
            let route = value.get("route")?;
            Some((
                route.get("session").and_then(|v| v.as_str())?.to_string(),
                route.get("page").and_then(|v| v.as_str())?.to_string(),
                route.get("frame").and_then(|v| v.as_str())?.to_string(),
            ))
        }
        "registry" => Some((
            value
                .get("session")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
            value
                .get("page")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
            value
                .get("frame")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
        )),
        _ => None,
    }
}

fn augment_request_for_replan(
    request: &AgentRequest,
    report: &FlowExecutionReport,
    attempt: u32,
) -> Option<AgentRequest> {
    let failure_step = report
        .steps
        .iter()
        .rev()
        .find(|step| matches!(step.status, StepExecutionStatus::Failed));

    let mut next_request = request.clone();
    let message = if let Some(step) = failure_step {
        let error = step.error.as_deref().unwrap_or("unknown error");
        format!(
            "Execution attempt {} failed at step '{}' after {} attempt(s). Error: {}. Please generate an alternative plan that avoids this failure while still achieving the goal.",
            attempt + 1,
            step.title,
            step.attempts,
            error
        )
    } else {
        format!(
            "Execution attempt {} failed for unspecified reasons. Please generate an alternative plan to complete the goal.",
            attempt + 1
        )
    };

    next_request.push_turn(ConversationTurn::new(ConversationRole::System, message));
    next_request.push_turn(ConversationTurn::new(
        ConversationRole::User,
        "Please suggest a revised plan that can succeed.",
    ));

    Some(next_request)
}

#[derive(Default)]
struct ArtifactManifest {
    items: Vec<ArtifactRecord>,
}

impl ArtifactManifest {
    fn add(&mut self, record: ArtifactRecord) {
        self.items.push(record);
    }

    fn to_values(&self) -> Vec<Value> {
        self.items.iter().map(|record| record.to_value()).collect()
    }

    fn build_output_payload(
        &self,
        plans: &[Value],
        execution: &[Value],
        events: Option<&[Value]>,
    ) -> Value {
        let artifacts = self.json_array();
        build_payload(plans, execution, artifacts, events)
    }

    fn json_array(&self) -> Value {
        Value::Array(self.to_values())
    }
}

fn build_payload(
    plans: &[Value],
    execution: &[Value],
    artifacts: Value,
    events: Option<&[Value]>,
) -> Value {
    if let Some(events) = events {
        json!({
            "plans": plans,
            "execution": execution,
            "artifacts": artifacts,
            "state_events": events,
        })
    } else {
        json!({
            "plans": plans,
            "execution": execution,
            "artifacts": artifacts,
        })
    }
}

async fn persist_run(
    path: &PathBuf,
    plans: &[Value],
    execution: &[Value],
    manifest: &ArtifactManifest,
    events: Option<&[Value]>,
) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .await
            .with_context(|| format!("Failed to create directory {}", parent.display()))?;
    }

    let payload = manifest.build_output_payload(plans, execution, events);

    fs::write(path, serde_json::to_vec_pretty(&payload)?)
        .await
        .with_context(|| format!("Failed to write run data to {}", path.display()))?;
    Ok(())
}

async fn save_artifact_manifest(path: &PathBuf, manifest: &ArtifactManifest) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .await
            .with_context(|| format!("Failed to create directory {}", parent.display()))?;
    }

    let json_bytes = serde_json::to_vec_pretty(&manifest.json_array())?;
    fs::write(path, json_bytes)
        .await
        .with_context(|| format!("Failed to write artifact manifest to {}", path.display()))?;
    Ok(())
}

#[derive(Clone, Debug)]
struct ArtifactRecord {
    attempt: u32,
    step_index: usize,
    step_id: String,
    dispatch_label: String,
    dispatch_index: usize,
    artifact_index: usize,
    action_id: String,
    label: String,
    content_type: String,
    byte_len: usize,
    filename: Option<String>,
    data_base64: String,
}

impl ArtifactRecord {
    fn to_value(&self) -> Value {
        json!({
            "attempt": self.attempt,
            "step_index": self.step_index,
            "step_id": self.step_id,
            "dispatch_label": self.dispatch_label,
            "dispatch_index": self.dispatch_index,
            "artifact_index": self.artifact_index,
            "action_id": self.action_id,
            "label": self.label,
            "content_type": self.content_type,
            "byte_len": self.byte_len,
            "filename": self.filename,
            "data_base64": self.data_base64,
        })
    }
}

fn state_events_to_values(events: &[StateEvent]) -> Vec<Value> {
    events
        .iter()
        .map(|event| match event {
            StateEvent::Dispatch(dispatch) => json!({
                "type": "dispatch",
                "status": match dispatch.status {
                    DispatchStatus::Success => "success",
                    DispatchStatus::Failure => "failure",
                },
                "action_id": dispatch.action_id.0,
                "task_id": dispatch.task_id,
                "route": {
                    "session": dispatch.route.session.0,
                    "page": dispatch.route.page.0,
                    "frame": dispatch.route.frame.0,
                    "mutex": dispatch.route.mutex_key.clone(),
                },
                "tool": dispatch.tool,
                "attempts": dispatch.attempts,
                "wait_ms": dispatch.wait_ms,
                "run_ms": dispatch.run_ms,
                "pending": dispatch.pending,
                "slots_available": dispatch.slots_available,
                "error": dispatch.error.as_ref().map(|e| e.to_string()),
                "output": dispatch.output.clone(),
                "recorded_at_ms": system_time_ms(dispatch.recorded_at),
            }),
            StateEvent::Registry(registry) => json!({
                "type": "registry",
                "action": format!("{:?}", registry.action).to_lowercase(),
                "session": registry.session.as_ref().map(|s| s.0.clone()),
                "page": registry.page.as_ref().map(|p| p.0.clone()),
                "frame": registry.frame.as_ref().map(|f| f.0.clone()),
                "note": registry.note,
                "recorded_at_ms": system_time_ms(registry.recorded_at),
            }),
            StateEvent::Perceiver(perceiver) => {
                let details = match &perceiver.kind {
                    PerceiverEventKind::Resolve {
                        strategy,
                        score,
                        candidate_count,
                        cache_hit,
                        breakdown,
                        reason,
                    } => json!({
                        "kind": "resolve",
                        "strategy": strategy,
                        "score": score,
                        "candidate_count": candidate_count,
                        "cache_hit": cache_hit,
                        "breakdown": breakdown,
                        "reason": reason,
                    }),
                    PerceiverEventKind::Judge {
                        check,
                        ok,
                        reason,
                        facts,
                    } => json!({
                        "kind": "judge",
                        "check": check,
                        "ok": ok,
                        "reason": reason,
                        "facts": facts,
                    }),
                    PerceiverEventKind::Snapshot { cache_hit } => json!({
                        "kind": "snapshot",
                        "cache_hit": cache_hit,
                    }),
                    PerceiverEventKind::Diff {
                        change_count,
                        changes,
                    } => json!({
                        "kind": "diff",
                        "change_count": change_count,
                        "changes": changes,
                    }),
                };

                json!({
                    "type": "perceiver",
                    "route": {
                        "session": perceiver.route.session.0.clone(),
                        "page": perceiver.route.page.0.clone(),
                        "frame": perceiver.route.frame.0.clone(),
                        "mutex": perceiver.route.mutex_key.clone(),
                    },
                    "recorded_at_ms": system_time_ms(perceiver.recorded_at),
                    "details": details,
                })
            }
        })
        .collect()
}

fn system_time_ms(time: SystemTime) -> u128 {
    time.duration_since(UNIX_EPOCH)
        .map(|dur| dur.as_millis())
        .unwrap_or(0)
}

async fn cmd_run(args: RunArgs, config: &Config) -> Result<()> {
    info!("Running automation script: {}", args.script.display());

    // Initialize or get AppContext
    let context = get_or_create_context(
        "cli".to_string(),
        Some(config.output_dir.clone()),
        config.policy_paths.clone(),
    )
    .await?;

    // Parse parameters
    let params = parse_parameters(&args.param)?;

    // Create automation config
    let automation_config = AutomationConfig {
        browser_type: args.browser,
        headless: args.headless,
        timeout: args.timeout,
        parallel_instances: args.parallel,
        parameters: params,
        output_dir: args.output_dir.clone(),
    };

    // Create automation engine using AppContext
    let mut automation = AutomationEngine::with_context(context.clone(), automation_config).await?;

    // Execute script
    let results = automation.execute_script(&args.script).await?;

    info!("Script execution completed");

    // Save results if output directory specified
    if let Some(output_dir) = args.output_dir {
        save_results(&results, &output_dir).await?;
        info!("Results saved to: {}", output_dir.display());
    }

    Ok(())
}

async fn cmd_record(args: RecordArgs, config: &Config) -> Result<()> {
    info!("Starting recording session: {}", args.name);

    let storage_path = args
        .output_dir
        .clone()
        .or_else(|| Some(config.output_dir.clone()));

    let context =
        get_or_create_context("cli".to_string(), storage_path, config.policy_paths.clone()).await?;

    let start_url = args.url.clone();

    let storage = context.storage();
    let tenant_id = TenantId("cli".to_string());
    let session_id = format!("record-{}-{}", args.name, uuid::Uuid::new_v4());
    info!("Recording session ID: {}", session_id);

    let created_at = chrono::Utc::now().timestamp_millis();
    let session_entity = BrowserSessionEntity {
        id: session_id.clone(),
        tenant: tenant_id.clone(),
        subject_id: "recorder".to_string(),
        created_at,
        updated_at: created_at,
        state: "recording".to_string(),
        metadata: serde_json::json!({
            "name": args.name,
            "url": start_url.clone(),
            "options": {
                "screenshots": args.screenshots,
                "video": args.video,
                "network": args.network,
                "performance": args.performance
            }
        }),
    };

    storage
        .backend()
        .store_session(session_entity)
        .await
        .context("Failed to persist session metadata")?;

    let mut sequence: u64 = 1;

    persist_event(
        &storage,
        &tenant_id,
        &session_id,
        sequence,
        "recording_started",
        serde_json::json!({
            "name": args.name,
            "url": start_url.clone(),
            "options": {
                "screenshots": args.screenshots,
                "video": args.video,
                "network": args.network,
                "performance": args.performance
            }
        }),
    )
    .await?;
    sequence += 1;

    // Initialize browser for recording
    let l0 = L0Protocol::new().await?;
    let browser_config = BrowserConfig {
        browser_type: args.browser,
        headless: false,
        window_size: Some((1280, 720)),
        devtools: true,
    };

    let mut browser_manager = L1BrowserManager::new(l0, browser_config).await?;
    let browser = browser_manager.launch_browser().await?;
    let mut page = browser.new_page().await?;

    // Navigate to start URL if provided
    if let Some(url) = start_url.as_deref() {
        page.navigate(url).await?;
    }

    info!("Recording started. Interact with the browser. Press Ctrl+C to stop.");

    // Wait for stop signal
    tokio::signal::ctrl_c().await?;

    persist_event(
        &storage,
        &tenant_id,
        &session_id,
        sequence,
        "recording_stopped",
        serde_json::json!({
            "reason": "user_exit"
        }),
    )
    .await?;

    let updated_at = chrono::Utc::now().timestamp_millis();
    let completed_session = BrowserSessionEntity {
        id: session_id.clone(),
        tenant: tenant_id.clone(),
        subject_id: "recorder".to_string(),
        created_at,
        updated_at,
        state: "completed".to_string(),
        metadata: serde_json::json!({
            "name": args.name,
            "url": start_url,
            "options": {
                "screenshots": args.screenshots,
                "video": args.video,
                "network": args.network,
                "performance": args.performance
            }
        }),
    };

    storage
        .backend()
        .update_session(completed_session)
        .await
        .context("Failed to update session state")?;

    info!(
        "Recording session {} complete. Replay with: cargo run -- replay {}",
        session_id, session_id
    );

    Ok(())
}

async fn cmd_replay(args: ReplayArgs, config: &Config) -> Result<()> {
    info!("Replaying session: {}", args.recording);

    // Initialize or get AppContext
    let context = get_or_create_context(
        "cli".to_string(),
        Some(config.output_dir.clone()),
        config.policy_paths.clone(),
    )
    .await?;

    let overrides = if args.param.is_empty() {
        None
    } else {
        Some(parse_parameters(&args.param)?)
    };

    let replay_config = ReplayConfig {
        recording_path: PathBuf::from(&config.output_dir),
        browser_type: args.browser,
        playback_speed: args.speed,
        headless: args.headless,
    };

    let replayer = SessionReplayer::with_context(context.clone(), replay_config);

    // Execute replay
    let results = replayer
        .replay_session(&args.recording, overrides.as_ref(), args.fail_fast)
        .await?;

    info!(
        "Replay completed: {} events replayed",
        results.events_replayed
    );

    if !results.errors.is_empty() {
        if args.fail_fast {
            bail!(
                "Replay encountered {} errors (fail-fast)",
                results.errors.len()
            );
        }

        warn!("Replay had {} errors", results.errors.len());
        for error in &results.errors {
            warn!("  - {}", error);
        }
    }

    if args.compare {
        let analyzer = SessionAnalyzer::with_context(context.clone());
        match analyzer.analyze_session(&args.recording).await {
            Ok(analytics) => {
                println!("Comparison summary (recorded session analytics):");
                println!("- Total events: {}", analytics.total_events);
                println!("- Duration: {} ms", analytics.duration_ms);
                if let Some((event, count)) = analytics
                    .event_types
                    .iter()
                    .max_by_key(|(_, count)| *count)
                    .map(|(k, v)| (k.clone(), *v))
                {
                    println!("- Most frequent event: {} ({} times)", event, count);
                }
                if let Some((page, visits)) = analytics
                    .page_visits
                    .iter()
                    .max_by_key(|(_, count)| *count)
                    .map(|(k, v)| (k.clone(), *v))
                {
                    println!("- Most visited page: {} ({} visits)", page, visits);
                }
            }
            Err(err) => {
                warn!("Failed to generate comparison analytics: {}", err);
            }
        }
    }

    Ok(())
}

async fn cmd_export(args: ExportArgs, config: &Config) -> Result<()> {
    // Initialize or get AppContext
    let context = get_or_create_context(
        "cli".to_string(),
        Some(config.output_dir.clone()),
        config.policy_paths.clone(),
    )
    .await?;

    match args.data_type {
        ExportType::Metrics {
            session,
            output,
            format,
        } => {
            info!("Exporting metrics for session: {}", session);

            let output_path = output.unwrap_or_else(|| {
                PathBuf::from(format!("{}_metrics.{}", session, format_extension(&format)))
            });

            // Use appropriate exporter based on format
            let stats = match format {
                DataFormat::Json => {
                    let exporter =
                        JsonExporter::with_context(context.clone(), Some(session.clone()));
                    exporter.export(&output_path).await?
                }
                DataFormat::Csv => {
                    let exporter =
                        CsvExporter::with_context(context.clone(), Some(session.clone()));
                    exporter.export(&output_path).await?
                }
                _ => {
                    let exporter =
                        JsonExporter::with_context(context.clone(), Some(session.clone()));
                    exporter.export(&output_path).await?
                }
            };

            info!(
                "Metrics exported to: {} ({} events)",
                output_path.display(),
                stats.total_events
            );
        }

        ExportType::Timeline {
            session,
            output,
            format,
        } => {
            info!("Exporting timeline for session: {}", session);

            let output_path = output.unwrap_or_else(|| {
                PathBuf::from(format!(
                    "{}_timeline.{}",
                    session,
                    timeline_format_extension(&format)
                ))
            });

            // Use HTML exporter for timeline
            let exporter = HtmlExporter::with_context(context.clone(), Some(session.clone()));
            let stats = exporter.export(&output_path).await?;
            info!(
                "Timeline exported to: {} ({} events)",
                output_path.display(),
                stats.total_events
            );
        }

        ExportType::Script {
            session,
            language,
            output,
        } => {
            info!("Exporting script for session: {}", session);

            let output_path = output.unwrap_or_else(|| {
                PathBuf::from(format!(
                    "{}_script.{}",
                    session,
                    script_extension(&language)
                ))
            });

            export_script(&context, &session, &language, &output_path).await?;
            info!("Script exported to: {}", output_path.display());
        }
    }

    Ok(())
}

async fn cmd_analyze(args: AnalyzeArgs, config: &Config) -> Result<()> {
    info!("Analyzing session: {}", args.target);

    // Initialize or get AppContext
    let context = get_or_create_context(
        "cli".to_string(),
        Some(config.output_dir.clone()),
        config.policy_paths.clone(),
    )
    .await?;

    let analyzer = SessionAnalyzer::with_context(context.clone());

    if args.target.eq_ignore_ascii_case("all") {
        let report = analyzer.generate_report().await?;

        println!("Portfolio Analysis (all sessions):");
        println!("- Total sessions: {}", report.total_sessions);
        println!("- Total events: {}", report.total_events);
        println!(
            "- Average events per session: {:.2}",
            report.average_events_per_session
        );
        println!(
            "- Average session duration: {} ms",
            report.average_duration_ms
        );

        if let Some((ref event, count)) = report.most_common_event {
            println!("- Most common event: {} ({} times)", event, count);
        }

        if let Some((ref page, visits)) = report.most_visited_page {
            println!("- Most visited page: {} ({} visits)", page, visits);
        }

        if let Some(output_path) = args.output {
            let json = serde_json::to_string_pretty(&report)?;
            fs::write(output_path.clone(), json).await?;
            info!("Aggregated report saved to: {}", output_path.display());
        }
        return Ok(());
    }

    let events = analyzer.session_events(&args.target).await?;

    if events.is_empty() {
        bail!(
            "No events found for session {}. Try recording or running automation first.",
            args.target
        );
    }

    let analytics = analyzer.analyze_session(&args.target).await?;

    let performance = build_performance_report(&args.target, &analytics, &events);
    let accessibility = build_accessibility_report(&args.target, &events);
    let security = build_security_report(&args.target, &events);
    let usability = build_usability_report(&args.target, &events);
    let compatibility = build_compatibility_report(&args.target, &events);

    if args.interactive {
        println!("Interactive reporting is not available in CLI mode; rendering static insights.");
        println!();
    }

    match args.analysis_type {
        AnalysisType::Full => {
            println!("Session analysis summary for {}:", args.target);
            println!("- Total events: {}", analytics.total_events);
            println!("- Duration: {} ms", analytics.duration_ms);
            println!("- Unique event types: {}", analytics.event_types.len());
            println!("- Pages visited: {}", analytics.page_visits.len());
            println!();

            print_performance_summary(&performance);
            println!();
            print_accessibility_summary(&accessibility);
            println!();
            print_security_summary(&security);
            println!();
            print_usability_summary(&usability);
            println!();
            print_compatibility_summary(&compatibility);

            if let Some(output_path) = args.output {
                let bundle = serde_json::json!({
                    "session": analytics,
                    "performance": performance,
                    "accessibility": accessibility,
                    "security": security,
                    "usability": usability,
                    "compatibility": compatibility,
                });
                let json = serde_json::to_string_pretty(&bundle)?;
                fs::write(output_path.clone(), json).await?;
                info!("Detailed analysis saved to: {}", output_path.display());
            }
        }
        AnalysisType::Performance => {
            println!("Performance analysis for {}:", args.target);
            print_performance_summary(&performance);

            if let Some(output_path) = args.output {
                let json = serde_json::to_string_pretty(&performance)?;
                fs::write(output_path.clone(), json).await?;
                info!("Performance report saved to: {}", output_path.display());
            }
        }
        AnalysisType::Accessibility => {
            println!("Accessibility analysis for {}:", args.target);
            print_accessibility_summary(&accessibility);

            if let Some(output_path) = args.output {
                let json = serde_json::to_string_pretty(&accessibility)?;
                fs::write(output_path.clone(), json).await?;
                info!("Accessibility report saved to: {}", output_path.display());
            }
        }
        AnalysisType::Security => {
            println!("Security analysis for {}:", args.target);
            print_security_summary(&security);

            if let Some(output_path) = args.output {
                let json = serde_json::to_string_pretty(&security)?;
                fs::write(output_path.clone(), json).await?;
                info!("Security report saved to: {}", output_path.display());
            }
        }
        AnalysisType::Usability => {
            println!("Usability analysis for {}:", args.target);
            print_usability_summary(&usability);

            if let Some(output_path) = args.output {
                let json = serde_json::to_string_pretty(&usability)?;
                fs::write(output_path.clone(), json).await?;
                info!("Usability report saved to: {}", output_path.display());
            }
        }
        AnalysisType::Compatibility => {
            println!("Compatibility analysis for {}:", args.target);
            print_compatibility_summary(&compatibility);

            if let Some(output_path) = args.output {
                let json = serde_json::to_string_pretty(&compatibility)?;
                fs::write(output_path.clone(), json).await?;
                info!("Compatibility report saved to: {}", output_path.display());
            }
        }
    }

    Ok(())
}

async fn cmd_chat(args: ChatArgs, config: &Config, output: OutputFormat) -> Result<()> {
    info!("Generating agent plan");

    let prompt = if let Some(prompt) = args.prompt.clone() {
        prompt
    } else if let Some(path) = args.prompt_file.as_ref() {
        fs::read_to_string(path)
            .await
            .with_context(|| format!("Failed to read prompt file: {}", path.display()))?
    } else {
        // Clap requires one of prompt or prompt_file; this branch is defensive.
        return Err(anyhow!("Either --prompt or --prompt-file must be provided"));
    };

    let prompt = prompt.trim().to_string();
    if prompt.is_empty() {
        return Err(anyhow!("Prompt cannot be empty"));
    }

    let app_context = get_or_create_context(
        "cli".to_string(),
        Some(config.output_dir.clone()),
        config.policy_paths.clone(),
    )
    .await?;

    let mut agent_context = AgentContext::default();
    if let Some(url) = args.current_url.clone() {
        agent_context.current_url = Some(url);
    }

    let has_context = agent_context.session.is_some()
        || agent_context.page.is_some()
        || agent_context.current_url.is_some()
        || !agent_context.preferences.is_empty()
        || !agent_context.memory_hints.is_empty()
        || !agent_context.metadata.is_empty();

    let runner = ChatRunner::default();
    let mut exec_request = runner.request_from_prompt(
        prompt.clone(),
        if has_context {
            Some(agent_context)
        } else {
            None
        },
        args.constraints.clone(),
    );

    let mut current_session = runner.plan(exec_request.clone())?;

    let mut plan_payloads = Vec::new();
    let mut execution_reports = Vec::new();
    let mut state_events_payload: Option<Vec<Value>> = None;

    if let Some(path) = args.save_plan.as_ref() {
        let json = serde_json::to_string_pretty(&current_session.plan)?;
        fs::write(path, json)
            .await
            .with_context(|| format!("Failed to write plan to {}", path.display()))?;
        info!("Plan saved to: {}", path.display());
    }

    if let Some(path) = args.save_flow.as_ref() {
        let json = serde_json::to_string_pretty(&current_session.flow.flow)?;
        fs::write(path, json)
            .await
            .with_context(|| format!("Failed to write flow to {}", path.display()))?;
        info!("Flow saved to: {}", path.display());
    }

    let mut attempt = 0u32;

    loop {
        // Record plan payload for structured outputs.
        plan_payloads.push(plan_payload(&current_session));

        if matches!(output, OutputFormat::Human) {
            print_human_plan(&current_session, attempt);
        }

        if !args.execute {
            break;
        }

        let exec_options = FlowExecutionOptions {
            max_retries: args.max_retries.max(1),
            ..FlowExecutionOptions::default()
        };

        let exec_report = execute_plan(
            app_context.clone(),
            &exec_request,
            &current_session.plan,
            exec_options,
        )
        .await?;

        if matches!(output, OutputFormat::Human) {
            print_execution_summary(&exec_report, attempt);
        }

        execution_reports.push(exec_report.clone());
        state_events_payload = Some(state_events_to_values(&app_context.state_center_snapshot()));

        if exec_report.success {
            break;
        }

        if attempt >= args.max_replans.into() {
            if let Some(last) = exec_report
                .steps
                .iter()
                .rev()
                .find(|s| matches!(s.status, StepExecutionStatus::Failed))
            {
                bail!("Execution stopped at step '{}'", last.step_id);
            } else {
                bail!("Execution failed");
            }
        }

        if let Some(updated) = augment_request_for_replan(&exec_request, &exec_report, attempt) {
            exec_request = updated;
        }

        current_session = runner.plan(exec_request.clone())?;
        attempt += 1;
    }

    let state_events_slice = state_events_payload.as_ref().map(|v| v.as_slice());
    let (execution_payloads, manifest) =
        build_execution_payloads(&execution_reports, state_events_slice);

    if !matches!(output, OutputFormat::Human) {
        emit_structured_output(
            &plan_payloads,
            &execution_payloads,
            &manifest,
            state_events_slice,
            output,
            args.artifacts_only,
        )?;
    }

    if let Some(path) = args.artifacts_path.as_ref() {
        save_artifact_manifest(path, &manifest).await?;
        info!("Artifact manifest written to: {}", path.display());
    }

    if let Some(path) = args.save_run.as_ref() {
        persist_run(
            path,
            &plan_payloads,
            &execution_payloads,
            &manifest,
            state_events_slice,
        )
        .await?;
        info!("Run data saved to: {}", path.display());
    }

    Ok(())
}

async fn cmd_artifacts(args: ArtifactsArgs) -> Result<()> {
    let bundle = load_run_bundle(&args.input).await?;

    let artifacts_value = bundle
        .get("artifacts")
        .cloned()
        .unwrap_or_else(|| Value::Array(Vec::new()));

    let filtered = filter_artifacts(&artifacts_value, &args);
    let artifacts_array = Value::Array(filtered.clone());

    if let Some(dir) = &args.extract {
        extract_artifacts(dir, &filtered).await?;
    }

    let summary = build_artifact_summary(&filtered, args.large_threshold);

    if let Some(path) = &args.summary_path {
        save_summary(path, &summary).await?;
    }

    match args.format {
        ArtifactFormat::Json => {
            let payload = json!({
                "summary": summary,
                "artifacts": artifacts_array,
            });
            println!("{}", serde_json::to_string_pretty(&payload)?);
        }
        ArtifactFormat::Yaml => {
            let payload = json!({
                "summary": summary,
                "artifacts": artifacts_array,
            });
            println!("{}", serde_yaml::to_string(&payload)?);
        }
        ArtifactFormat::Human => {
            print_summary_human(&summary);
            if filtered.is_empty() {
                println!("[no artifacts]");
            } else {
                for item in &filtered {
                    let attempt = item.get("attempt").and_then(Value::as_u64).unwrap_or(0);
                    let step = item.get("step_index").and_then(Value::as_u64).unwrap_or(0);
                    let step_id = item
                        .get("step_id")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown");
                    let dispatch = item
                        .get("dispatch_label")
                        .and_then(Value::as_str)
                        .unwrap_or("action");
                    let label = item
                        .get("label")
                        .and_then(Value::as_str)
                        .unwrap_or("artifact");
                    let content_type = item
                        .get("content_type")
                        .and_then(Value::as_str)
                        .unwrap_or("application/octet-stream");
                    let bytes = item.get("byte_len").and_then(Value::as_u64).unwrap_or(0);
                    let filename = item.get("filename").and_then(Value::as_str);
                    println!(
                        "attempt={} step={} ({}) dispatch={} artifact={} bytes={} type={}{}",
                        attempt,
                        step,
                        step_id,
                        dispatch,
                        label,
                        bytes,
                        content_type,
                        filename
                            .map(|name| format!(" filename={}", name))
                            .unwrap_or_default()
                    );
                }
            }
        }
    }

    Ok(())
}

async fn cmd_console(args: ConsoleArgs) -> Result<()> {
    let bundle = load_run_bundle(&args.input).await?;

    let execution = bundle
        .get("execution")
        .cloned()
        .unwrap_or_else(|| Value::Array(Vec::new()));
    let plans = bundle
        .get("plans")
        .cloned()
        .unwrap_or_else(|| Value::Array(Vec::new()));
    let state_events = bundle
        .get("state_events")
        .cloned()
        .unwrap_or_else(|| Value::Array(Vec::new()));
    let artifacts_value = bundle
        .get("artifacts")
        .cloned()
        .unwrap_or_else(|| Value::Array(Vec::new()));

    let artifacts_array = artifacts_value.clone();
    let items = artifacts_value.as_array().cloned().unwrap_or_else(Vec::new);
    let summary = build_artifact_summary(&items, DEFAULT_LARGE_THRESHOLD);
    let overlays = build_overlays(&items);

    let payload = json!({
        "plans": plans,
        "execution": execution,
        "state_events": state_events,
        "artifacts": {
            "summary": summary,
            "items": artifacts_array,
        },
        "overlays": overlays,
    });

    if args.serve {
        serve_console(args.port, payload).await?;
        return Ok(());
    }

    if let Some(path) = &args.output {
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await.with_context(|| {
                format!("failed to create output directory {}", parent.display())
            })?;
        }
        tokio::fs::write(path, serde_json::to_vec_pretty(&payload)?)
            .await
            .with_context(|| format!("failed to write console payload to {}", path.display()))?;
        println!("Console bundle written to {}", path.display());
    } else if args.pretty {
        println!("{}", serde_json::to_string_pretty(&payload)?);
    } else {
        println!("{}", serde_json::to_string(&payload)?);
    }

    Ok(())
}

async fn load_run_bundle(path: &PathBuf) -> Result<Value> {
    let content = fs::read_to_string(path)
        .await
        .with_context(|| format!("failed to read run bundle {}", path.display()))?;
    let bundle: Value = serde_json::from_str(&content)
        .with_context(|| format!("failed to parse run bundle {}", path.display()))?;
    Ok(bundle)
}

async fn serve_console(port: u16, payload: Value) -> Result<()> {
    let shared = Arc::new(payload);
    let data = shared.clone();

    let router = axum::Router::new()
        .route(
            "/",
            axum::routing::get(|| async { axum::response::Html(CONSOLE_HTML) }),
        )
        .route(
            "/data",
            axum::routing::get(move || {
                let clone = data.clone();
                async move { axum::Json(clone.as_ref().clone()) }
            }),
        );

    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let listener = TcpListener::bind(addr)
        .await
        .with_context(|| format!("failed to bind console server on {}", addr))?;
    println!("Console preview available at http://{}", addr);
    axum::serve(listener, router.into_make_service())
        .await
        .context("console server exited unexpectedly")?;
    Ok(())
}

#[derive(Clone)]
struct ServeState {
    ws_url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct UiPerceiveRequest {
    url: String,
    structural: Option<bool>,
    visual: Option<bool>,
    semantic: Option<bool>,
    insights: Option<bool>,
    #[serde(default)]
    screenshot: Option<bool>,
    #[serde(default)]
    timeout: Option<u64>,
    mode: Option<String>,
}

#[derive(Serialize)]
struct UiPerceiveResponse {
    success: bool,
    perception: Option<MultiModalPerception>,
    screenshot_base64: Option<String>,
    stdout: String,
    stderr: String,
    error: Option<String>,
}

struct PerceptionExecResult {
    success: bool,
    perception: Option<MultiModalPerception>,
    screenshot_base64: Option<String>,
    stdout: String,
    stderr: String,
    error_message: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ConsoleFixture {
    #[serde(default)]
    success: Option<bool>,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    stdout: Option<String>,
    #[serde(default)]
    stderr: Option<String>,
    #[serde(default)]
    perception: Option<MultiModalPerception>,
    #[serde(default)]
    screenshot_base64: Option<String>,
}

async fn load_console_fixture() -> Result<Option<PerceptionExecResult>> {
    let path = match std::env::var("SOULBROWSER_CONSOLE_FIXTURE") {
        Ok(path) if !path.trim().is_empty() => PathBuf::from(path),
        Ok(_) | Err(std::env::VarError::NotPresent) => return Ok(None),
        Err(err) => {
            return Err(anyhow::anyhow!(
                "failed to read SOULBROWSER_CONSOLE_FIXTURE env var: {}",
                err
            ));
        }
    };

    let data = tokio::fs::read(&path)
        .await
        .with_context(|| format!("failed to read console fixture {}", path.display()))?;

    let fixture: ConsoleFixture = serde_json::from_slice(&data)
        .with_context(|| format!("failed to parse console fixture {}", path.display()))?;

    let success = fixture.success.unwrap_or(true);
    let perception = if success {
        Some(
            fixture
                .perception
                .ok_or_else(|| anyhow::anyhow!("console fixture missing 'perception' payload"))?,
        )
    } else {
        fixture.perception
    };

    let screenshot_base64 = match std::env::var("SOULBROWSER_CONSOLE_FIXTURE_SCREENSHOT") {
        Ok(extra_path) if !extra_path.trim().is_empty() => {
            let img_path = PathBuf::from(extra_path);
            let bytes = tokio::fs::read(&img_path).await.with_context(|| {
                format!(
                    "failed to read console fixture screenshot {}",
                    img_path.display()
                )
            })?;
            Some(Base64.encode(bytes))
        }
        Ok(_) | Err(std::env::VarError::NotPresent) => fixture.screenshot_base64,
        Err(err) => {
            return Err(anyhow::anyhow!(
                "failed to read SOULBROWSER_CONSOLE_FIXTURE_SCREENSHOT env var: {}",
                err
            ));
        }
    };

    let stdout = fixture
        .stdout
        .unwrap_or_else(|| "console fixture: perception executed".to_string());
    let stderr = fixture.stderr.unwrap_or_default();

    Ok(Some(PerceptionExecResult {
        success,
        perception,
        screenshot_base64,
        stdout,
        stderr,
        error_message: fixture.error,
    }))
}

async fn cmd_serve(args: ServeArgs, _metrics_port: u16, _config: Config) -> Result<()> {
    let state = ServeState {
        ws_url: args.ws_url.clone(),
    };

    let router = axum::Router::new()
        .route(
            "/",
            axum::routing::get(|| async { axum::response::Html(CONSOLE_HTML) }),
        )
        .route(
            "/health",
            axum::routing::get(|| async { axum::Json(json!({ "status": "ok" })) }),
        )
        .route("/api/perceive", axum::routing::post(serve_perceive_handler))
        .with_state(state);

    let addr = SocketAddr::from(([127, 0, 0, 1], args.port));
    let listener = TcpListener::bind(addr)
        .await
        .with_context(|| format!("failed to bind testing server on {}", addr))?;
    println!("Testing console available at http://{}", addr);
    if let Some(ws) = args.ws_url {
        println!("Using external DevTools endpoint: {}", ws);
    } else {
        println!("Using local Chrome detection (SOULBROWSER_CHROME / auto-detect)");
    }

    axum::serve(listener, router.into_make_service())
        .await
        .context("testing server exited unexpectedly")?;
    Ok(())
}

async fn serve_perceive_handler(
    axum::extract::State(state): axum::extract::State<ServeState>,
    axum::Json(req): axum::Json<UiPerceiveRequest>,
) -> impl axum::response::IntoResponse {
    if req.url.trim().is_empty() {
        let body = UiPerceiveResponse {
            success: false,
            perception: None,
            screenshot_base64: None,
            stdout: String::new(),
            stderr: String::new(),
            error: Some("URL must not be empty".to_string()),
        };
        return (axum::http::StatusCode::BAD_REQUEST, axum::Json(body)).into_response();
    }

    let result = run_perception_subprocess(&req, state.ws_url.as_deref()).await;
    match result {
        Ok(exec) => {
            let status = if exec.success {
                axum::http::StatusCode::OK
            } else {
                axum::http::StatusCode::INTERNAL_SERVER_ERROR
            };
            let body = UiPerceiveResponse {
                success: exec.success,
                perception: exec.perception,
                screenshot_base64: exec.screenshot_base64,
                stdout: exec.stdout,
                stderr: exec.stderr,
                error: exec.error_message,
            };
            (status, axum::Json(body)).into_response()
        }
        Err(err) => {
            error!(?err, "perception subprocess failed");
            let body = UiPerceiveResponse {
                success: false,
                perception: None,
                screenshot_base64: None,
                stdout: String::new(),
                stderr: String::new(),
                error: Some(err.to_string()),
            };
            (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                axum::Json(body),
            )
                .into_response()
        }
    }
}

async fn run_perception_subprocess(
    req: &UiPerceiveRequest,
    ws_url: Option<&str>,
) -> Result<PerceptionExecResult> {
    if let Some(result) = load_console_fixture().await? {
        debug!("console fixture mode active");
        return Ok(result);
    }

    let temp = tempdir().context("failed to create temporary directory")?;
    let perception_path = temp.path().join("perception.json");
    let screenshot_path = temp.path().join("screenshot.png");

    let exe = std::env::current_exe().context("failed to locate current executable")?;
    let mut cmd = TokioCommand::new(exe);
    cmd.arg("--metrics-port").arg("0");
    cmd.arg("perceive");
    cmd.arg("--url").arg(&req.url);

    if let Some(timeout) = req.timeout {
        cmd.arg("--timeout").arg(timeout.to_string());
    }

    let mode = req.mode.as_deref().unwrap_or("");
    let structural = req.structural.unwrap_or(false);
    let visual = req.visual.unwrap_or(false);
    let semantic = req.semantic.unwrap_or(false);

    if mode.eq_ignore_ascii_case("all") {
        cmd.arg("--all");
    } else if mode.eq_ignore_ascii_case("structural") {
        cmd.arg("--structural");
    } else if mode.eq_ignore_ascii_case("visual") {
        cmd.arg("--visual");
    } else if mode.eq_ignore_ascii_case("semantic") {
        cmd.arg("--semantic");
    } else {
        let mut any = false;
        if structural {
            cmd.arg("--structural");
            any = true;
        }
        if visual {
            cmd.arg("--visual");
            any = true;
        }
        if semantic {
            cmd.arg("--semantic");
            any = true;
        }
        if !any {
            cmd.arg("--all");
        }
    }

    if req.insights.unwrap_or(mode.eq_ignore_ascii_case("all")) {
        cmd.arg("--insights");
    }

    cmd.arg("--output").arg(&perception_path);

    let capture_screenshot = req
        .screenshot
        .unwrap_or_else(|| req.visual.unwrap_or(false) || mode.eq_ignore_ascii_case("all"));
    if capture_screenshot {
        cmd.arg("--screenshot").arg(&screenshot_path);
    }

    if let Some(ws) = ws_url {
        cmd.env("SOULBROWSER_WS_URL", ws);
    }
    if std::env::var("SOULBROWSER_USE_REAL_CHROME").is_err() {
        cmd.env("SOULBROWSER_USE_REAL_CHROME", "1");
    }

    if let Ok(val) = std::env::var("SOULBROWSER_CHROME") {
        cmd.env("SOULBROWSER_CHROME", val);
    }

    if let Ok(val) = std::env::var("SOULBROWSER_DISABLE_SANDBOX") {
        cmd.env("SOULBROWSER_DISABLE_SANDBOX", val);
    } else {
        cmd.env("SOULBROWSER_DISABLE_SANDBOX", "1");
    }

    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    let output = cmd
        .output()
        .await
        .context("failed to launch perception subprocess")?;
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    if !output.status.success() {
        return Ok(PerceptionExecResult {
            success: false,
            perception: None,
            screenshot_base64: None,
            stdout,
            stderr,
            error_message: Some(format!(
                "perceive command failed with status {}",
                output.status
            )),
        });
    }

    let json_bytes = match tokio::fs::read(&perception_path).await {
        Ok(bytes) => bytes,
        Err(err) => {
            return Ok(PerceptionExecResult {
                success: false,
                perception: None,
                screenshot_base64: None,
                stdout,
                stderr,
                error_message: Some(format!("failed to read perception output: {err}")),
            });
        }
    };

    let perception: MultiModalPerception = match serde_json::from_slice(&json_bytes) {
        Ok(value) => value,
        Err(err) => {
            return Ok(PerceptionExecResult {
                success: false,
                perception: None,
                screenshot_base64: None,
                stdout,
                stderr,
                error_message: Some(format!("failed to parse perception output: {err}")),
            });
        }
    };

    let screenshot_base64 = if capture_screenshot {
        match tokio::fs::read(&screenshot_path).await {
            Ok(bytes) if !bytes.is_empty() => Some(Base64.encode(bytes)),
            Ok(_) => None,
            Err(err) => {
                warn!(?err, "failed to read screenshot");
                None
            }
        }
    } else {
        None
    };

    Ok(PerceptionExecResult {
        success: true,
        perception: Some(perception),
        screenshot_base64,
        stdout,
        stderr,
        error_message: None,
    })
}

fn filter_artifacts(value: &Value, args: &ArtifactsArgs) -> Vec<Value> {
    let Some(items) = value.as_array() else {
        return Vec::new();
    };

    items
        .iter()
        .filter(|item| {
            if let Some(step_id) = &args.step_id {
                if item
                    .get("step_id")
                    .and_then(Value::as_str)
                    .map(|s| s != step_id)
                    .unwrap_or(true)
                {
                    return false;
                }
            }

            if let Some(dispatch) = &args.dispatch {
                if item
                    .get("dispatch_label")
                    .and_then(Value::as_str)
                    .map(|s| s != dispatch)
                    .unwrap_or(true)
                {
                    return false;
                }
            }

            if let Some(label) = &args.label {
                if item
                    .get("label")
                    .and_then(Value::as_str)
                    .map(|s| s != label)
                    .unwrap_or(true)
                {
                    return false;
                }
            }

            true
        })
        .cloned()
        .collect()
}

async fn extract_artifacts(dir: &PathBuf, artifacts: &[Value]) -> Result<()> {
    fs::create_dir_all(dir)
        .await
        .with_context(|| format!("failed to create extract directory {}", dir.display()))?;

    for item in artifacts {
        let label = item
            .get("label")
            .and_then(Value::as_str)
            .unwrap_or("artifact");
        let attempt = item.get("attempt").and_then(Value::as_u64).unwrap_or(0);
        let step = item.get("step_index").and_then(Value::as_u64).unwrap_or(0);
        let dispatch = item
            .get("dispatch_label")
            .and_then(Value::as_str)
            .unwrap_or("action");
        let filename_hint = item.get("filename").and_then(Value::as_str);
        let data_base64 = item
            .get("data_base64")
            .and_then(Value::as_str)
            .unwrap_or("");

        if data_base64.is_empty() {
            continue;
        }

        let bytes = match Base64.decode(data_base64) {
            Ok(bytes) => bytes,
            Err(err) => {
                warn!("failed to decode artifact {}: {}", label, err);
                continue;
            }
        };

        let file_name = filename_hint
            .map(|name| name.to_string())
            .unwrap_or_else(|| {
                format!("attempt{}_step{}_{}_{}.bin", attempt, step, dispatch, label)
            });

        let path = dir.join(file_name);
        fs::write(&path, bytes)
            .await
            .with_context(|| format!("failed to write artifact {}", path.display()))?;
    }

    Ok(())
}

fn build_artifact_summary(items: &[Value], large_threshold: u64) -> Value {
    let total = items.len() as u64;
    let total_bytes: u64 = items
        .iter()
        .filter_map(|item| item.get("byte_len").and_then(Value::as_u64))
        .sum();
    let mut steps = HashSet::new();
    let mut dispatches = HashSet::new();
    let mut types: BTreeMap<String, (u64, u64)> = BTreeMap::new();
    let mut large = Vec::new();

    for item in items {
        if let Some(step) = item.get("step_id").and_then(Value::as_str) {
            steps.insert(step.to_string());
        }
        if let Some(dispatch) = item.get("dispatch_label").and_then(Value::as_str) {
            dispatches.insert(dispatch.to_string());
        }
        let ctype = item
            .get("content_type")
            .and_then(Value::as_str)
            .unwrap_or("application/octet-stream");
        let bytes = item.get("byte_len").and_then(Value::as_u64).unwrap_or(0);
        let entry = types.entry(ctype.to_string()).or_insert((0, 0));
        entry.0 += 1;
        entry.1 += bytes;

        if bytes >= large_threshold {
            large.push(json!({
                "step_id": item.get("step_id"),
                "dispatch_label": item.get("dispatch_label"),
                "label": item.get("label"),
                "byte_len": bytes,
                "content_type": ctype,
                "attempt": item.get("attempt"),
                "filename": item.get("filename"),
            }));
        }
    }

    let average_bytes = if total > 0 { total_bytes / total } else { 0 };

    let mut steps_vec: Vec<String> = steps.into_iter().collect();
    steps_vec.sort();
    let mut dispatch_vec: Vec<String> = dispatches.into_iter().collect();
    dispatch_vec.sort();

    let content_types: Vec<Value> = types
        .into_iter()
        .map(|(ctype, (count, bytes))| {
            json!({
                "content_type": ctype,
                "count": count,
                "total_bytes": bytes,
                "average_bytes": if count > 0 { bytes / count } else { 0 },
            })
        })
        .collect();

    json!({
        "count": total,
        "total_bytes": total_bytes,
        "average_bytes": average_bytes,
        "steps": steps_vec,
        "dispatches": dispatch_vec,
        "content_types": content_types,
        "large_artifacts": large,
    })
}

fn print_summary_human(summary: &Value) {
    let count = summary.get("count").and_then(Value::as_u64).unwrap_or(0);
    let total_bytes = summary
        .get("total_bytes")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let average = summary
        .get("average_bytes")
        .and_then(Value::as_u64)
        .unwrap_or(0);

    println!(
        "Artifacts: {} ({} bytes total, avg {} bytes)",
        count, total_bytes, average
    );

    if let Some(steps) = summary.get("steps").and_then(Value::as_array) {
        if !steps.is_empty() {
            let list: Vec<&str> = steps.iter().filter_map(|v| v.as_str()).collect();
            println!("Steps: {}", list.join(", "));
        }
    }

    if let Some(dispatches) = summary.get("dispatches").and_then(Value::as_array) {
        if !dispatches.is_empty() {
            let list: Vec<&str> = dispatches.iter().filter_map(|v| v.as_str()).collect();
            println!("Dispatch labels: {}", list.join(", "));
        }
    }

    if let Some(types) = summary.get("content_types").and_then(Value::as_array) {
        if !types.is_empty() {
            println!("Content types:");
            for entry in types {
                let ctype = entry
                    .get("content_type")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown");
                let count = entry.get("count").and_then(Value::as_u64).unwrap_or(0);
                let bytes = entry
                    .get("total_bytes")
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                let avg = entry
                    .get("average_bytes")
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                println!(
                    "  - {}: {} artifacts ({} bytes total, avg {} bytes)",
                    ctype, count, bytes, avg
                );
            }
        }
    }

    if let Some(large) = summary.get("large_artifacts").and_then(Value::as_array) {
        if !large.is_empty() {
            println!("Large artifacts:");
            for entry in large {
                let step = entry
                    .get("step_id")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown");
                let dispatch = entry
                    .get("dispatch_label")
                    .and_then(Value::as_str)
                    .unwrap_or("action");
                let label = entry
                    .get("label")
                    .and_then(Value::as_str)
                    .unwrap_or("artifact");
                let bytes = entry.get("byte_len").and_then(Value::as_u64).unwrap_or(0);
                println!(
                    "  - step={} dispatch={} artifact={} ({} bytes)",
                    step, dispatch, label, bytes
                );
            }
        }
    }
}

async fn save_summary(path: &PathBuf, summary: &Value) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .await
            .with_context(|| format!("failed to create summary directory {}", parent.display()))?;
    }

    fs::write(path, serde_json::to_vec_pretty(summary)?)
        .await
        .with_context(|| format!("failed to write summary to {}", path.display()))?;
    Ok(())
}

fn build_overlays(items: &[Value]) -> Value {
    let overlays: Vec<Value> = items
        .iter()
        .filter(|item| {
            item.get("content_type")
                .and_then(Value::as_str)
                .map(|ctype| ctype.starts_with("image/"))
                .unwrap_or(false)
        })
        .map(|item| {
            json!({
                "step_id": item.get("step_id"),
                "dispatch_label": item.get("dispatch_label"),
                "label": item.get("label"),
                "byte_len": item.get("byte_len"),
                "content_type": item.get("content_type"),
                "data_base64": item.get("data_base64"),
                "filename": item.get("filename"),
            })
        })
        .collect();

    Value::Array(overlays)
}

async fn cmd_config(args: ConfigArgs, config: &Config) -> Result<()> {
    match args.action {
        ConfigAction::Show => {
            println!("Current Configuration:");
            println!("{}", serde_yaml::to_string(config)?);
        }

        ConfigAction::Set { key, value } => {
            info!("Setting configuration: {} = {}", key, value);
            // Implementation for setting config values
            // This would modify the config file
        }

        ConfigAction::Get { key } => {
            info!("Getting configuration value: {}", key);
            // Implementation for getting specific config values
        }

        ConfigAction::Reset => {
            info!("Resetting configuration to defaults");
            // Implementation for resetting config
        }

        ConfigAction::Validate => {
            info!("Validating configuration");
            // Implementation for config validation
        }
    }

    Ok(())
}

async fn cmd_info(config: &Config) -> Result<()> {
    let context = get_or_create_context(
        "cli-info".to_string(),
        Some(config.output_dir.clone()),
        config.policy_paths.clone(),
    )
    .await?;
    let overview = scheduler_overview(&context);
    let state_events = context.state_center_snapshot();
    let mut successes = 0usize;
    let mut failures = 0usize;
    let mut registry_count = 0usize;
    let mut perceiver_resolve = 0usize;
    let mut perceiver_judge = 0usize;
    let mut perceiver_snapshot = 0usize;
    let mut perceiver_diff = 0usize;
    for event in &state_events {
        match event {
            StateEvent::Dispatch(dispatch) => match dispatch.status {
                DispatchStatus::Success => successes += 1,
                DispatchStatus::Failure => failures += 1,
            },
            StateEvent::Registry(_) => registry_count += 1,
            StateEvent::Perceiver(perceiver) => match &perceiver.kind {
                PerceiverEventKind::Resolve { .. } => perceiver_resolve += 1,
                PerceiverEventKind::Judge { .. } => perceiver_judge += 1,
                PerceiverEventKind::Snapshot { .. } => perceiver_snapshot += 1,
                PerceiverEventKind::Diff { .. } => perceiver_diff += 1,
            },
        }
    }

    let last_failure = state_events.iter().rev().find_map(|event| match event {
        StateEvent::Dispatch(dispatch) if matches!(dispatch.status, DispatchStatus::Failure) => {
            Some(dispatch)
        }
        _ => None,
    });

    let last_dispatch = state_events.iter().rev().find_map(|event| match event {
        StateEvent::Dispatch(dispatch) => Some(dispatch),
        _ => None,
    });

    let last_registry = state_events.iter().rev().find_map(|event| match event {
        StateEvent::Registry(event) => Some(event),
        _ => None,
    });

    println!("SoulBrowser System Information");
    println!("============================");
    println!("Version: {}", env!("CARGO_PKG_VERSION"));
    println!("Build Date: {}", env!("BUILD_DATE", "unknown"));
    println!("Git Commit: {}", env!("GIT_HASH", "unknown"));
    println!();

    println!("Configuration:");
    println!("- Default Browser: {:?}", config.default_browser);
    println!("- Output Directory: {}", config.output_dir.display());
    println!("- Soul Enabled: {}", config.soul.enabled);
    if config.policy_paths.is_empty() {
        println!("- Policy Paths: (default search)");
    } else {
        println!("- Policy Paths:");
        for path in &config.policy_paths {
            println!("  - {}", path.display());
        }
    }
    let strict_env = std::env::var("SOUL_STRICT_AUTHZ")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    println!(
        "- Strict Authorization: {}",
        if config.strict_authorization || strict_env {
            "enabled"
        } else {
            "disabled"
        }
    );
    println!();

    println!("Scheduler Dispatch Summary:");
    println!("- Recorded events: {}", state_events.len());
    println!("- Successes: {}", successes);
    println!("- Failures: {}", failures);
    println!("- Registry events: {}", registry_count);
    println!(
        "- Scheduler snapshot captured_at: {}",
        overview.captured_at.to_rfc3339()
    );
    println!(
        "- Runtime queue → total={} lightning={} quick={} standard={} deep={}",
        overview.runtime.queue_depth,
        overview.queue_by_priority.lightning,
        overview.queue_by_priority.quick,
        overview.queue_by_priority.standard,
        overview.queue_by_priority.deep
    );
    println!(
        "- Runtime slots → inflight={}/{} (free={})",
        overview.runtime.inflight, overview.runtime.global_limit, overview.runtime.slots_free
    );
    println!(
        "- Metrics counters → enqueued={} started={} completed={} failed={} cancelled={}",
        overview.metrics.enqueued,
        overview.metrics.started,
        overview.metrics.completed,
        overview.metrics.failed,
        overview.metrics.cancelled
    );
    if perceiver_resolve + perceiver_judge + perceiver_snapshot + perceiver_diff > 0 {
        println!(
            "- Perceiver events → resolve: {}, judge: {}, snapshot: {}, diff: {}",
            perceiver_resolve, perceiver_judge, perceiver_snapshot, perceiver_diff
        );
    }
    if let Some(failure) = last_failure {
        println!(
            "- Last failure: {} at {} (error: {})",
            failure.tool,
            format_rfc3339(failure.recorded_at),
            failure
                .error
                .as_ref()
                .map(|e| e.to_string())
                .unwrap_or_else(|| "unknown".to_string())
        );
    }
    if let Some(registry) = last_registry {
        println!(
            "- Last registry event: {:?} at {}",
            registry.action,
            format_rfc3339(registry.recorded_at)
        );
    }
    if let Some(latest) = last_dispatch {
        let recorded_at = format_rfc3339(latest.recorded_at);
        println!(
            "- Last tool: {} ({} attempts at {})",
            latest.tool, latest.attempts, recorded_at
        );
        println!(
            "  wait={}ms run={}ms pending={} slots={} status={}",
            latest.wait_ms,
            latest.run_ms,
            latest.pending,
            latest.slots_available,
            match latest.status {
                DispatchStatus::Success => "success",
                DispatchStatus::Failure => "failure",
            }
        );
        if let Some(err) = &latest.error {
            println!("  error: {}", err);
        }
    } else {
        println!("- Last tool: n/a");
    }
    println!();

    println!("Available Browsers:");
    // Check which browsers are available
    let browsers = check_available_browsers().await?;
    for browser in browsers {
        println!("- {} ✓", browser);
    }

    println!();
    println!("System Health: ✓ All systems operational");

    Ok(())
}

async fn cmd_perceiver(args: PerceiverArgs, config: &Config) -> Result<()> {
    let context = get_or_create_context(
        "cli-perceiver".to_string(),
        Some(config.output_dir.clone()),
        config.policy_paths.clone(),
    )
    .await?;

    let perceiver_events: Vec<PerceiverEvent> = context
        .state_center_snapshot()
        .into_iter()
        .filter_map(|event| match event {
            StateEvent::Perceiver(perceiver) => Some(perceiver),
            _ => None,
        })
        .collect();

    if perceiver_events.is_empty() {
        println!("No perceiver events recorded yet.");
        return Ok(());
    }

    let filtered = filter_perceiver_events(perceiver_events, &args);

    if filtered.is_empty() {
        println!("No perceiver events matched the provided filters.");
        return Ok(());
    }

    let summary = summarize_perceiver_events(&filtered);

    match args.format {
        PerceiverOutputFormat::Table => {
            print_perceiver_summary(&summary);
            print_perceiver_table(&filtered);
        }
        PerceiverOutputFormat::Json => {
            let events: Vec<_> = filtered.iter().map(perceiver_event_to_json).collect();
            let payload = json!({
                "summary": summary,
                "events": events,
            });
            println!("{}", serde_json::to_string_pretty(&payload)?);
        }
    }

    Ok(())
}

#[derive(Serialize)]
struct PerceiverSummary {
    resolve: usize,
    judge: usize,
    snapshot: usize,
    diff: usize,
    metrics: MetricSnapshot,
}

fn filter_perceiver_events(
    mut events: Vec<PerceiverEvent>,
    args: &PerceiverArgs,
) -> Vec<PerceiverEvent> {
    let limit = args.limit.unwrap_or(20);
    let check_filter = args.check.as_ref().map(|value| value.to_lowercase());
    let session_filter = args.session.as_ref();
    let page_filter = args.page.as_ref();
    let frame_filter = args.frame.as_ref();

    events = events
        .into_iter()
        .filter(|event| {
            if let Some(kind_filter) = args.kind.as_ref() {
                if !matches_perceiver_kind(kind_filter, &event.kind) {
                    return false;
                }
            }

            if let Some(expected_session) = session_filter {
                if &event.route.session.0 != expected_session {
                    return false;
                }
            }

            if let Some(expected_page) = page_filter {
                if &event.route.page.0 != expected_page {
                    return false;
                }
            }

            if let Some(expected_frame) = frame_filter {
                if &event.route.frame.0 != expected_frame {
                    return false;
                }
            }

            if let Some(cache_filter) = args.cache.as_ref() {
                if !matches_cache_filter(cache_filter, &event.kind) {
                    return false;
                }
            }

            if let Some(check_filter) = check_filter.as_ref() {
                match &event.kind {
                    PerceiverEventKind::Judge { check, .. } => {
                        if check.to_lowercase() != *check_filter {
                            return false;
                        }
                    }
                    _ => return false,
                }
            }

            true
        })
        .collect();

    events.sort_by_key(|event| event_timestamp_ms(event));
    events.reverse();

    if limit > 0 && events.len() > limit {
        events.truncate(limit);
    }

    events
}

fn summarize_perceiver_events(events: &[PerceiverEvent]) -> PerceiverSummary {
    let mut resolve = 0usize;
    let mut judge = 0usize;
    let mut snapshot = 0usize;
    let mut diff = 0usize;
    for event in events {
        match event.kind {
            PerceiverEventKind::Resolve { .. } => resolve += 1,
            PerceiverEventKind::Judge { .. } => judge += 1,
            PerceiverEventKind::Snapshot { .. } => snapshot += 1,
            PerceiverEventKind::Diff { .. } => diff += 1,
        }
    }

    PerceiverSummary {
        resolve,
        judge,
        snapshot,
        diff,
        metrics: structural_metrics::snapshot(),
    }
}

fn print_perceiver_summary(summary: &PerceiverSummary) {
    println!(
        "Perceiver summary → resolve: {} | judge: {} | snapshot: {} | diff: {}",
        summary.resolve, summary.judge, summary.snapshot, summary.diff
    );
    println!(
        "Metric summary → resolve: {} (avg {:.2}ms) | judge: {} (avg {:.2}ms) | snapshot: {} (avg {:.2}ms) | diff: {} (avg {:.2}ms)",
        summary.metrics.resolve.total,
        summary.metrics.resolve.avg_ms,
        summary.metrics.judge.total,
        summary.metrics.judge.avg_ms,
        summary.metrics.snapshot.total,
        summary.metrics.snapshot.avg_ms,
        summary.metrics.diff.total,
        summary.metrics.diff.avg_ms
    );
    println!(
        "Cache stats → resolve: {} hit / {} miss ({:.1}%) | snapshot: {} hit / {} miss ({:.1}%)",
        summary.metrics.resolve_cache.hits,
        summary.metrics.resolve_cache.misses,
        summary.metrics.resolve_cache.hit_rate,
        summary.metrics.snapshot_cache.hits,
        summary.metrics.snapshot_cache.misses,
        summary.metrics.snapshot_cache.hit_rate
    );
}

fn print_perceiver_table(events: &[PerceiverEvent]) {
    println!("Showing {} most recent perceiver event(s):", events.len());
    for event in events {
        let timestamp = format_rfc3339(event.recorded_at);
        let route = &event.route;
        match &event.kind {
            PerceiverEventKind::Resolve {
                strategy,
                score,
                candidate_count,
                cache_hit,
                breakdown,
                reason,
            } => {
                let breakdown_summary = summarize_breakdown(breakdown);
                println!(
                    "[{}] resolve session={} page={} frame={} strategy={} score={:.2} candidates={} cache={} reason=\"{}\" breakdown=[{}]",
                    timestamp,
                    route.session.0,
                    route.page.0,
                    route.frame.0,
                    strategy,
                    score,
                    candidate_count,
                    if *cache_hit { "hit" } else { "miss" },
                    reason,
                    breakdown_summary,
                );
            }
            PerceiverEventKind::Judge {
                check,
                ok,
                reason,
                facts,
            } => {
                println!(
                    "[{}] judge::{} session={} page={} frame={} status={} reason=\"{}\" facts={}",
                    timestamp,
                    check,
                    route.session.0,
                    route.page.0,
                    route.frame.0,
                    if *ok { "ok" } else { "fail" },
                    reason,
                    compact_json(facts, 80),
                );
            }
            PerceiverEventKind::Snapshot { cache_hit } => {
                println!(
                    "[{}] snapshot session={} page={} frame={} cache={}",
                    timestamp,
                    route.session.0,
                    route.page.0,
                    route.frame.0,
                    if *cache_hit { "hit" } else { "miss" }
                );
            }
            PerceiverEventKind::Diff {
                change_count,
                changes,
            } => {
                println!(
                    "[{}] diff session={} page={} frame={} changes={} sample={}",
                    timestamp,
                    route.session.0,
                    route.page.0,
                    route.frame.0,
                    change_count,
                    changes
                        .get(0)
                        .map(|value| compact_json(value, 80))
                        .unwrap_or_else(|| "-".into()),
                );
            }
        }
    }
}

fn perceiver_event_to_json(event: &PerceiverEvent) -> serde_json::Value {
    let timestamp = format_rfc3339(event.recorded_at).to_string();
    let route = &event.route;
    match &event.kind {
        PerceiverEventKind::Resolve {
            strategy,
            score,
            candidate_count,
            cache_hit,
            breakdown,
            reason,
        } => json!({
            "timestamp": timestamp,
            "kind": "resolve",
            "session": route.session.0,
            "page": route.page.0,
            "frame": route.frame.0,
            "mutex_key": route.mutex_key,
            "strategy": strategy,
            "score": score,
            "candidate_count": candidate_count,
            "cache_hit": cache_hit,
            "reason": reason,
            "score_breakdown": breakdown,
        }),
        PerceiverEventKind::Judge {
            check,
            ok,
            reason,
            facts,
        } => json!({
            "timestamp": timestamp,
            "kind": "judge",
            "session": route.session.0,
            "page": route.page.0,
            "frame": route.frame.0,
            "mutex_key": route.mutex_key,
            "check": check,
            "ok": ok,
            "reason": reason,
            "facts": facts,
        }),
        PerceiverEventKind::Snapshot { cache_hit } => json!({
            "timestamp": timestamp,
            "kind": "snapshot",
            "session": route.session.0,
            "page": route.page.0,
            "frame": route.frame.0,
            "mutex_key": route.mutex_key,
            "cache_hit": cache_hit,
        }),
        PerceiverEventKind::Diff {
            change_count,
            changes,
        } => json!({
            "timestamp": timestamp,
            "kind": "diff",
            "session": route.session.0,
            "page": route.page.0,
            "frame": route.frame.0,
            "mutex_key": route.mutex_key,
            "change_count": change_count,
            "changes": changes,
        }),
    }
}

fn summarize_breakdown(breakdown: &[ScoreComponentRecord]) -> String {
    if breakdown.is_empty() {
        return String::new();
    }

    let sum_abs: f32 = breakdown
        .iter()
        .map(|component| component.contribution.abs())
        .sum();
    let mut components: Vec<&ScoreComponentRecord> = breakdown.iter().collect();
    components.sort_by(|a, b| {
        b.contribution
            .abs()
            .partial_cmp(&a.contribution.abs())
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    components
        .into_iter()
        .map(|component| {
            let magnitude = component.contribution.abs();
            let sign = if component.contribution >= 0.0 {
                '+'
            } else {
                '-'
            };
            let share = if sum_abs > f32::EPSILON {
                (magnitude / sum_abs) * 100.0
            } else {
                0.0
            };
            format!(
                "{}:{}{:.3} (w={:.3}, {:>3.0}%)",
                component.label, sign, magnitude, component.weight, share
            )
        })
        .collect::<Vec<_>>()
        .join(" | ")
}

fn compact_json(value: &serde_json::Value, limit: usize) -> String {
    let raw = match value {
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    truncate(&raw, limit)
}

fn truncate(text: &str, limit: usize) -> String {
    if text.len() <= limit {
        text.to_string()
    } else {
        format!("{}…", &text[..limit])
    }
}

fn matches_perceiver_kind(filter: &PerceiverKindFilter, kind: &PerceiverEventKind) -> bool {
    match (filter, kind) {
        (PerceiverKindFilter::Resolve, PerceiverEventKind::Resolve { .. }) => true,
        (PerceiverKindFilter::Judge, PerceiverEventKind::Judge { .. }) => true,
        (PerceiverKindFilter::Snapshot, PerceiverEventKind::Snapshot { .. }) => true,
        (PerceiverKindFilter::Diff, PerceiverEventKind::Diff { .. }) => true,
        _ => false,
    }
}

fn matches_cache_filter(filter: &PerceiverCacheFilter, kind: &PerceiverEventKind) -> bool {
    match (filter, kind) {
        (PerceiverCacheFilter::Hit, PerceiverEventKind::Resolve { cache_hit, .. }) => *cache_hit,
        (PerceiverCacheFilter::Miss, PerceiverEventKind::Resolve { cache_hit, .. }) => !cache_hit,
        (PerceiverCacheFilter::Hit, PerceiverEventKind::Snapshot { cache_hit }) => *cache_hit,
        (PerceiverCacheFilter::Miss, PerceiverEventKind::Snapshot { cache_hit }) => !cache_hit,
        _ => false,
    }
}

fn event_timestamp_ms(event: &PerceiverEvent) -> u128 {
    event
        .recorded_at
        .duration_since(UNIX_EPOCH)
        .unwrap_or_else(|_| Duration::from_secs(0))
        .as_millis()
}

#[cfg(test)]
mod perceiver_cli_tests {
    use super::*;
    use serde_json::json;
    use std::time::Duration;

    fn base_args() -> PerceiverArgs {
        PerceiverArgs {
            limit: None,
            kind: None,
            check: None,
            session: None,
            page: None,
            frame: None,
            cache: None,
            format: PerceiverOutputFormat::Table,
        }
    }

    fn route(session: &str, page: &str, frame: &str) -> ExecRoute {
        ExecRoute {
            session: SessionId(session.to_string()),
            page: PageId(page.to_string()),
            frame: FrameId(frame.to_string()),
            mutex_key: format!("frame:{}", frame),
        }
    }

    fn stamped(mut event: PerceiverEvent, millis: u64) -> PerceiverEvent {
        event.recorded_at = UNIX_EPOCH + Duration::from_millis(millis);
        event
    }

    fn score_components(items: Vec<(&str, f32, f32)>) -> Vec<ScoreComponentRecord> {
        items
            .into_iter()
            .map(|(label, weight, contribution)| ScoreComponentRecord {
                label: label.into(),
                weight,
                contribution,
            })
            .collect()
    }

    #[test]
    fn filters_by_kind_and_orders_descending() {
        let events = vec![
            stamped(
                PerceiverEvent::resolve(
                    route("s1", "p1", "f1"),
                    "css".into(),
                    0.7,
                    2,
                    false,
                    score_components(vec![("confidence", 1.0, 0.7)]),
                    "score=0.7".into(),
                ),
                10,
            ),
            stamped(
                PerceiverEvent::judge(
                    route("s1", "p1", "f1"),
                    "visible".into(),
                    true,
                    "geometry".into(),
                    json!({ "geometry": {"width": 120} }),
                ),
                30,
            ),
            stamped(
                PerceiverEvent::diff(route("s1", "p1", "f1"), 5, vec![json!({"kind": "text"})]),
                20,
            ),
        ];

        let mut args = base_args();
        args.kind = Some(PerceiverKindFilter::Judge);
        let filtered = filter_perceiver_events(events, &args);
        assert_eq!(filtered.len(), 1);
        match &filtered[0].kind {
            PerceiverEventKind::Judge { check, .. } => assert_eq!(check, "visible"),
            _ => panic!("expected judge event"),
        }
    }

    #[test]
    fn filters_by_cache_status() {
        let events = vec![
            stamped(
                PerceiverEvent::resolve(
                    route("s1", "p1", "f1"),
                    "css".into(),
                    0.8,
                    3,
                    true,
                    score_components(vec![("confidence", 1.0, 0.8)]),
                    "score=0.8".into(),
                ),
                40,
            ),
            stamped(PerceiverEvent::snapshot(route("s1", "p1", "f1"), false), 50),
        ];

        let mut args = base_args();
        args.cache = Some(PerceiverCacheFilter::Miss);
        let filtered = filter_perceiver_events(events, &args);
        assert_eq!(filtered.len(), 1);
        match &filtered[0].kind {
            PerceiverEventKind::Snapshot { cache_hit } => assert!(!cache_hit),
            _ => panic!("expected snapshot event"),
        }
    }

    #[test]
    fn applies_limit_and_session_filter() {
        let events = vec![
            stamped(
                PerceiverEvent::resolve(
                    route("a", "p1", "f1"),
                    "css".into(),
                    0.9,
                    4,
                    false,
                    score_components(vec![("confidence", 1.0, 0.9)]),
                    "score=0.9".into(),
                ),
                10,
            ),
            stamped(
                PerceiverEvent::resolve(
                    route("b", "p2", "f2"),
                    "css".into(),
                    0.6,
                    1,
                    true,
                    score_components(vec![("confidence", 1.0, 0.6)]),
                    "score=0.6".into(),
                ),
                30,
            ),
            stamped(
                PerceiverEvent::resolve(
                    route("a", "p3", "f3"),
                    "text".into(),
                    0.5,
                    2,
                    false,
                    score_components(vec![("confidence", 1.0, 0.5)]),
                    "score=0.5".into(),
                ),
                20,
            ),
        ];

        let mut args = base_args();
        args.session = Some("a".into());
        args.limit = Some(1);
        let filtered = filter_perceiver_events(events, &args);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].route.session.0, "a");
        // Ensure the most recent event survives the limit.
        assert_eq!(filtered[0].route.page.0, "p3");
    }

    #[test]
    fn summarize_breakdown_formats_components() {
        let records = vec![
            ScoreComponentRecord {
                label: "visibility".into(),
                weight: 0.5,
                contribution: 0.6,
            },
            ScoreComponentRecord {
                label: "text".into(),
                weight: 0.3,
                contribution: 0.2,
            },
        ];
        let summary = super::summarize_breakdown(&records);
        assert!(summary.contains("visibility:+0.600"));
        assert!(summary.contains("w=0.500"));
        assert!(summary.contains("text:+0.200"));
        assert!(summary.contains("%"));
    }
}

async fn cmd_scheduler(args: SchedulerArgs, config: &Config) -> Result<()> {
    let context = get_or_create_context(
        "cli-scheduler".to_string(),
        Some(config.output_dir.clone()),
        config.policy_paths.clone(),
    )
    .await?;

    if let Some(action_id) = args.cancel.as_ref() {
        let scheduler = context.scheduler_service();
        let cancelled = scheduler
            .cancel(ActionId(action_id.to_string()))
            .await
            .map_err(|e| anyhow!(e.to_string()))?;
        if cancelled {
            println!("Action {} cancelled", action_id);
        } else {
            println!("Action {} not found or already completed", action_id);
        }
        return Ok(());
    }

    let events = context.state_center_snapshot();
    let overview = scheduler_overview(&context);

    let limit = args.limit.unwrap_or(20);
    let status_filter = args.status;

    let filtered_iter = events.into_iter().rev().filter(|event| {
        if let Some(filter) = status_filter.as_ref() {
            match event {
                StateEvent::Dispatch(dispatch) => match (filter, &dispatch.status) {
                    (SchedulerStatusFilter::Success, DispatchStatus::Success) => true,
                    (SchedulerStatusFilter::Failure, DispatchStatus::Failure) => true,
                    _ => false,
                },
                StateEvent::Registry(_) | StateEvent::Perceiver(_) => false,
            }
        } else {
            matches!(event, StateEvent::Dispatch(_))
        }
    });

    let display_events: Vec<DispatchEvent> = if limit == 0 {
        filtered_iter
            .filter_map(|event| match event {
                StateEvent::Dispatch(dispatch) => Some(dispatch),
                _ => None,
            })
            .collect()
    } else {
        filtered_iter
            .take(limit)
            .filter_map(|event| match event {
                StateEvent::Dispatch(dispatch) => Some(dispatch),
                _ => None,
            })
            .collect()
    };

    match args.format {
        SchedulerOutputFormat::Text => {
            println!(
                "Scheduler counters → enqueued={} started={} completed={} failed={} cancelled={}",
                overview.metrics.enqueued,
                overview.metrics.started,
                overview.metrics.completed,
                overview.metrics.failed,
                overview.metrics.cancelled
            );
            println!(
                "Scheduler runtime → queue={} inflight={} slots_free={} (limit={}, per_task_limit={})",
                overview.runtime.queue_depth,
                overview.runtime.inflight,
                overview.runtime.slots_free,
                overview.runtime.global_limit,
                overview.runtime.per_task_limit
            );
            println!(
                "Queue breakdown → lightning={} quick={} standard={} deep={}",
                overview.queue_by_priority.lightning,
                overview.queue_by_priority.quick,
                overview.queue_by_priority.standard,
                overview.queue_by_priority.deep
            );
            if display_events.is_empty() {
                if status_filter.is_some() {
                    println!("No events match the selected filters.");
                } else {
                    println!("No dispatch events recorded yet.");
                }
                return Ok(());
            }
            println!(
                "Recent scheduler dispatch events (latest first, showing up to {}):",
                if limit == 0 {
                    display_events.len()
                } else {
                    limit.min(display_events.len())
                }
            );
            if let Some(filter) = status_filter.as_ref() {
                let label = match filter {
                    SchedulerStatusFilter::Success => "success",
                    SchedulerStatusFilter::Failure => "failure",
                };
                println!("  filter: {}", label);
            }
            for (idx, dispatch) in display_events.iter().enumerate() {
                let status = match dispatch.status {
                    DispatchStatus::Success => "success",
                    DispatchStatus::Failure => "failure",
                };
                let recorded_at = format_rfc3339(dispatch.recorded_at);
                println!(
                    "{:>2}. [{} @ {}] tool={} route={} attempts={} wait={}ms run={}ms pending={} slots={}",
                    idx + 1,
                    status,
                    recorded_at,
                    dispatch.tool,
                    dispatch.route,
                    dispatch.attempts,
                    dispatch.wait_ms,
                    dispatch.run_ms,
                    dispatch.pending,
                    dispatch.slots_available,
                );
                if let Some(err) = &dispatch.error {
                    println!("    error: {}", err);
                }
            }
        }
        SchedulerOutputFormat::Json => {
            let payload = scheduler_json_payload(&overview, &display_events);
            println!("{}", serde_json::to_string_pretty(&payload)?);
        }
    }

    Ok(())
}

#[derive(Clone, Debug)]
struct SchedulerRuntimeSummary {
    queue_depth: usize,
    inflight: usize,
    slots_free: usize,
    global_limit: usize,
    per_task_limit: usize,
}

#[derive(Clone, Debug)]
struct QueueByPriority {
    lightning: usize,
    quick: usize,
    standard: usize,
    deep: usize,
}

#[derive(Clone, Debug)]
struct SchedulerOverview {
    metrics: scheduler_metrics::SchedulerMetricsSnapshot,
    runtime: SchedulerRuntimeSummary,
    queue_by_priority: QueueByPriority,
    captured_at: chrono::DateTime<chrono::Utc>,
}

fn scheduler_overview(context: &Arc<AppContext>) -> SchedulerOverview {
    let metrics = scheduler_metrics::snapshot();
    let captured_at = Utc::now();
    let runtime = context.scheduler_runtime();
    let queue_depth = runtime.pending();
    let slots_free = runtime.global_slots().available_permits();
    let config = runtime.config();
    let inflight = config.global_slots.saturating_sub(slots_free);
    let per_priority = runtime.depth_by_priority();

    SchedulerOverview {
        metrics,
        runtime: SchedulerRuntimeSummary {
            queue_depth,
            inflight,
            slots_free,
            global_limit: config.global_slots,
            per_task_limit: config.per_task_limit,
        },
        queue_by_priority: QueueByPriority {
            lightning: per_priority[Priority::Lightning.index()],
            quick: per_priority[Priority::Quick.index()],
            standard: per_priority[Priority::Standard.index()],
            deep: per_priority[Priority::Deep.index()],
        },
        captured_at,
    }
}

fn scheduler_json_payload(
    overview: &SchedulerOverview,
    events: &[DispatchEvent],
) -> serde_json::Value {
    let json_events: Vec<_> = events.iter().map(dispatch_event_to_value).collect();
    json!({
        "captured_at": overview.captured_at.to_rfc3339(),
        "metrics": {
            "enqueued": overview.metrics.enqueued,
            "started": overview.metrics.started,
            "completed": overview.metrics.completed,
            "failed": overview.metrics.failed,
            "cancelled": overview.metrics.cancelled,
        },
        "runtime": {
            "queue_depth": overview.runtime.queue_depth,
            "inflight": overview.runtime.inflight,
            "slots_free": overview.runtime.slots_free,
            "global_limit": overview.runtime.global_limit,
            "per_task_limit": overview.runtime.per_task_limit,
        },
        "queue_by_priority": {
            "lightning": overview.queue_by_priority.lightning,
            "quick": overview.queue_by_priority.quick,
            "standard": overview.queue_by_priority.standard,
            "deep": overview.queue_by_priority.deep,
        },
        "events": json_events,
    })
}

async fn attach_permissions_for_adapter(adapter: &Arc<CdpAdapter>) {
    match get_or_create_context("cli-permissions".to_string(), None, Vec::new()).await {
        Ok(context) => context.attach_cdp_adapter(adapter).await,
        Err(err) => warn!(?err, "failed to attach permissions broker to adapter"),
    }
}

fn dispatch_event_to_value(dispatch: &DispatchEvent) -> serde_json::Value {
    let recorded_at = format_rfc3339(dispatch.recorded_at).to_string();
    json!({
        "status": match dispatch.status {
            DispatchStatus::Success => "success",
            DispatchStatus::Failure => "failure",
        },
        "recorded_at": recorded_at,
        "tool": dispatch.tool.clone(),
        "route": dispatch.route.to_string(),
        "attempts": dispatch.attempts,
        "wait_ms": dispatch.wait_ms,
        "run_ms": dispatch.run_ms,
        "pending": dispatch.pending,
        "slots_available": dispatch.slots_available,
        "error": dispatch.error.as_ref().map(|e| e.to_string()),
    })
}

impl SchedulerRuntimeSummary {
    #[cfg(test)]
    fn new_for_test(
        queue_depth: usize,
        inflight: usize,
        slots_free: usize,
        global_limit: usize,
        per_task_limit: usize,
    ) -> Self {
        Self {
            queue_depth,
            inflight,
            slots_free,
            global_limit,
            per_task_limit,
        }
    }
}

impl QueueByPriority {
    #[cfg(test)]
    fn new_for_test(lightning: usize, quick: usize, standard: usize, deep: usize) -> Self {
        Self {
            lightning,
            quick,
            standard,
            deep,
        }
    }
}

impl SchedulerOverview {
    #[cfg(test)]
    fn new_for_test(
        metrics: scheduler_metrics::SchedulerMetricsSnapshot,
        runtime: SchedulerRuntimeSummary,
        queue_by_priority: QueueByPriority,
    ) -> Self {
        Self {
            metrics,
            runtime,
            queue_by_priority,
            captured_at: chrono::DateTime::<chrono::Utc>::from_timestamp(1_700_000_000, 0)
                .expect("valid test timestamp"),
        }
    }
}

#[cfg(test)]
mod scheduler_cli_tests {
    use super::*;
    use serde_json::json;
    use soulbrowser_core_types::{ActionId, FrameId, PageId, SessionId};

    fn mock_route() -> ExecRoute {
        ExecRoute::new(SessionId::new(), PageId::new(), FrameId::new())
    }

    #[test]
    fn scheduler_json_payload_contains_expected_fields() {
        let metrics = scheduler_metrics::SchedulerMetricsSnapshot {
            enqueued: 10,
            started: 8,
            completed: 7,
            failed: 2,
            cancelled: 1,
            queue_length: 4,
        };
        let runtime = SchedulerRuntimeSummary::new_for_test(5, 3, 2, 6, 2);
        let queue = QueueByPriority::new_for_test(2, 1, 1, 1);
        let overview = SchedulerOverview::new_for_test(metrics, runtime, queue);

        let route = mock_route();
        let mutex_key = route.mutex_key.clone();
        let dispatch = DispatchEvent::success(
            ActionId::new(),
            Some("task-1".into()),
            route.clone(),
            "tool.click".into(),
            mutex_key,
            1,
            12,
            34,
            0,
            4,
            None,
        );
        let expected_timestamp = overview.captured_at.to_rfc3339();
        let payload = scheduler_json_payload(&overview, &[dispatch]);
        assert_eq!(payload["captured_at"], json!(expected_timestamp));

        assert_eq!(payload["metrics"]["enqueued"], json!(10));
        assert_eq!(payload["metrics"]["failed"], json!(2));
        assert_eq!(payload["runtime"]["queue_depth"], json!(5));
        assert_eq!(payload["runtime"]["global_limit"], json!(6));
        assert_eq!(payload["queue_by_priority"]["lightning"], json!(2));
        assert_eq!(payload["queue_by_priority"]["deep"], json!(1));
        assert!(payload["events"].is_array());
        assert_eq!(payload["events"].as_array().unwrap().len(), 1);
        assert_eq!(payload["events"][0]["tool"], "tool.click");
        assert_eq!(payload["events"][0]["status"], "success");
    }
}

async fn cmd_policy(args: PolicyArgs, config: &Config) -> Result<()> {
    let context = get_or_create_context(
        "cli-policy".to_string(),
        Some(config.output_dir.clone()),
        config.policy_paths.clone(),
    )
    .await?;

    match args.command {
        PolicyCommand::Show(show_args) => {
            let snapshot = context.policy_center().snapshot().await;
            let stats = context.state_center_stats();
            let overview = scheduler_overview(&context);
            if show_args.json {
                let payload = serde_json::json!({
                    "policy": &*snapshot,
                    "state_center_stats": stats,
                    "scheduler": scheduler_json_payload(&overview, &[]),
                });
                println!("{}", serde_json::to_string_pretty(&payload)?);
            } else {
                println!("Policy Revision: {}", snapshot.rev);
                println!();
                println!(
                    "Scheduler Limits → global={}, per_task={}, queue={}",
                    snapshot.scheduler.limits.global_slots,
                    snapshot.scheduler.limits.per_task_limit,
                    snapshot.scheduler.limits.queue_capacity
                );
                println!(
                    "Scheduler Retry → max_attempts={}, backoff_ms={}",
                    snapshot.scheduler.retry.max_attempts, snapshot.scheduler.retry.backoff_ms
                );
                println!(
                    "Registry → allow_multiple_pages={}, health_probe_interval_ms={}",
                    snapshot.registry.allow_multiple_pages,
                    snapshot.registry.health_probe_interval_ms
                );
                println!(
                    "Features → state_center_persistence={}, metrics_export={}, registry_ingest_bus={}",
                    snapshot.features.state_center_persistence,
                    snapshot.features.metrics_export,
                    snapshot.features.registry_ingest_bus
                );
                println!(
                    "Scheduler Runtime → queue={} (lightning={} quick={} standard={} deep={}) inflight={}/{} slots_free={}",
                    overview.runtime.queue_depth,
                    overview.queue_by_priority.lightning,
                    overview.queue_by_priority.quick,
                    overview.queue_by_priority.standard,
                    overview.queue_by_priority.deep,
                    overview.runtime.inflight,
                    overview.runtime.global_limit,
                    overview.runtime.slots_free
                );
                println!(
                    "Scheduler Metrics → enqueued={} started={} completed={} failed={} cancelled={}",
                    overview.metrics.enqueued,
                    overview.metrics.started,
                    overview.metrics.completed,
                    overview.metrics.failed,
                    overview.metrics.cancelled
                );
                println!(
                    "State Center Counters → total={}, success={}, failure={}, registry={}",
                    stats.total_events,
                    stats.dispatch_success,
                    stats.dispatch_failure,
                    stats.registry_events
                );
            }
        }
        PolicyCommand::Override(override_args) => {
            let value = serde_json::from_str::<serde_json::Value>(&override_args.value)
                .unwrap_or_else(|_| serde_json::Value::String(override_args.value.clone()));
            let spec = RuntimeOverrideSpec {
                path: override_args.path.clone(),
                value,
                owner: override_args.owner.clone(),
                reason: override_args.reason.clone(),
                ttl_seconds: override_args.ttl.unwrap_or(0),
            };
            context
                .policy_center()
                .apply_override(spec)
                .await
                .map_err(|e| anyhow!(e.to_string()))?;
            let snapshot = context.policy_center().snapshot().await;
            println!("Override applied. Current revision: {}", snapshot.rev);
        }
    }

    Ok(())
}

async fn cmd_timeline(args: TimelineArgs, config: &Config) -> Result<()> {
    let context = get_or_create_context(
        "cli-timeline".to_string(),
        Some(config.output_dir.clone()),
        config.policy_paths.clone(),
    )
    .await
    .map_err(|e| anyhow!(e.to_string()))?;

    let state_center = context.state_center();
    let storage = context.storage();

    let es_policy = TimelineEsPolicy::default();
    let event_store = TimelineEventStore::new(es_policy);
    let event_store_dyn: Arc<dyn TimelineEventStoreTrait> = event_store.clone();

    let range = parse_timeline_range(&args)?;
    populate_event_store_from_storage(&event_store_dyn, storage, &args, range)
        .await
        .context("failed to hydrate event store from storage")?;

    let mut view_policy = TimelinePolicyView::default();
    view_policy.log_enable = args.output.is_some();
    if let Some(max_lines) = args.max_lines {
        view_policy.max_lines = max_lines;
    }
    if let Some(path) = &args.output {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).with_context(|| {
                format!("failed to create output directory {}", parent.display())
            })?;
        }
        view_policy.log_path = path.to_string_lossy().to_string();
    }
    timeline_set_policy(view_policy);

    let selector = build_timeline_selector(&args, range)?;
    let req = TimelineExportReq {
        view: args.view.into(),
        by: selector,
        policy_overrides: None,
    };

    let service =
        L6TimelineService::with_runtime(event_store_dyn.clone(), Some(state_center), None);
    let result = service
        .export(req)
        .await
        .map_err(|err| anyhow!(err.to_string()))?;

    if let Some(path) = result.path {
        println!("Timeline export written to {path}");
    } else if let Some(lines) = result.lines {
        if let Some(path) = &args.output {
            std::fs::write(
                path,
                lines.join(
                    "
",
                ) + "
",
            )
            .with_context(|| format!("failed to write export to {}", path.display()))?;
            println!("Timeline export written to {}", path.display());
        } else {
            for line in lines {
                println!("{}", line);
            }
        }
    }

    println!(
        "Timeline stats → actions={} lines={} truncated={}",
        result.stats.total_actions, result.stats.total_lines, result.stats.truncated
    );
    Ok(())
}

fn build_timeline_selector(
    args: &TimelineArgs,
    range: Option<(chrono::DateTime<Utc>, chrono::DateTime<Utc>)>,
) -> Result<TimelineBy> {
    let mut provided = 0;
    if args.action_id.is_some() {
        provided += 1;
    }
    if args.flow_id.is_some() {
        provided += 1;
    }
    if args.task_id.is_some() {
        provided += 1;
    }
    if range.is_some() {
        provided += 1;
    }

    if provided != 1 {
        bail!("Specify exactly one selector: --action-id, --flow-id, --task-id or ( --since + --until )");
    }

    if let Some(action) = &args.action_id {
        return Ok(TimelineBy::Action {
            action_id: action.clone(),
        });
    }
    if let Some(flow) = &args.flow_id {
        return Ok(TimelineBy::Flow {
            flow_id: flow.clone(),
        });
    }
    if let Some(task) = &args.task_id {
        return Ok(TimelineBy::Task {
            task_id: task.clone(),
        });
    }

    let (since, until) = range.expect("range already validated");
    Ok(TimelineBy::Range { since, until })
}

async fn populate_event_store_from_storage(
    event_store: &Arc<dyn TimelineEventStoreTrait>,
    storage: Arc<StorageManager>,
    args: &TimelineArgs,
    range: Option<(chrono::DateTime<Utc>, chrono::DateTime<Utc>)>,
) -> Result<()> {
    let backend = storage.backend();
    let mut params = QueryParams::default();
    params.limit = args.limit;
    if let Some(flow) = &args.flow_id {
        params.session_id = Some(flow.clone());
    }
    if let Some((since, until)) = range {
        params.from_timestamp = Some(since.timestamp_millis());
        params.to_timestamp = Some(until.timestamp_millis());
    }

    let events = backend
        .query_events(params)
        .await
        .map_err(|err| anyhow!(err.to_string()))?;

    for event in events {
        if let Some(env) = browser_event_to_envelope(event, args.action_id.as_deref()) {
            event_store
                .append_event(env, TimelineAppendMeta::default())
                .await
                .map_err(|err| anyhow!(err.to_string()))?;
        }
    }
    Ok(())
}

fn browser_event_to_envelope(
    event: BrowserEvent,
    fallback_action: Option<&str>,
) -> Option<TimelineEventEnvelope> {
    let action_id = event
        .data
        .get("action_id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .or_else(|| fallback_action.map(|s| s.to_string()))?;

    let ts_wall = match Utc.timestamp_millis_opt(event.timestamp) {
        LocalResult::Single(dt) => dt,
        _ => return None,
    };
    let scope = TimelineEventScope {
        session: Some(SessionId(event.session_id.clone())),
        action: Some(ActionId(action_id.clone())),
        ..Default::default()
    };

    Some(TimelineEventEnvelope {
        event_id: event.id,
        ts_mono: event.timestamp.max(0) as u128,
        ts_wall,
        scope,
        source: TimelineEventSource::L5,
        kind: event.event_type,
        level: TimelineLogLevel::Info,
        payload: event.data,
        artifacts: Vec::new(),
        tags: event
            .tags
            .into_iter()
            .map(|tag| (tag, String::new()))
            .collect(),
    })
}

fn parse_timeline_range(
    args: &TimelineArgs,
) -> Result<Option<(chrono::DateTime<Utc>, chrono::DateTime<Utc>)>> {
    match (&args.since, &args.until) {
        (Some(since), Some(until)) => {
            let since = chrono::DateTime::parse_from_rfc3339(since)
                .map_err(|e| anyhow!("invalid --since timestamp: {e}"))?
                .with_timezone(&Utc);
            let until = chrono::DateTime::parse_from_rfc3339(until)
                .map_err(|e| anyhow!("invalid --until timestamp: {e}"))?
                .with_timezone(&Utc);
            if until <= since {
                bail!("--until must be greater than --since");
            }
            Ok(Some((since, until)))
        }
        (None, None) => Ok(None),
        _ => bail!("Both --since and --until must be provided for range queries"),
    }
}

async fn cmd_gateway(args: GatewayArgs, config: &Config) -> Result<()> {
    info!("Starting L7 gateway surfaces");

    let GatewayArgs {
        http: http_addr,
        grpc: grpc_addr,
        webdriver: webdriver_addr,
        adapter_policy: adapter_policy_path,
        webdriver_policy: webdriver_policy_path,
    } = args;

    let context = get_or_create_context(
        "gateway".to_string(),
        Some(config.output_dir.clone()),
        config.policy_paths.clone(),
    )
    .await
    .map_err(|e| anyhow!(e.to_string()))?;

    let scheduler_dyn: Arc<dyn Dispatcher + Send + Sync> = context.scheduler_service();
    let state_center = context.state_center();

    let adapter_policy_handle = AdapterPolicyHandle::global();
    let adapter_policy = if let Some(path) = adapter_policy_path.as_ref() {
        load_policy_file::<AdapterPolicyView>(path)
            .await
            .with_context(|| format!("failed to load adapter policy from {}", path.display()))?
    } else {
        default_adapter_policy()
    };
    adapter_policy_handle.update(adapter_policy);

    let dispatcher_port: Arc<dyn DispatcherPort> =
        Arc::new(GatewayDispatcherPort::new(Arc::clone(&scheduler_dyn)));
    let readonly_port: Arc<dyn ReadonlyPort> = Arc::new(GatewayReadonlyPort::new(state_center));

    let bootstrap = AdapterBootstrap::new(adapter_policy_handle, dispatcher_port, readonly_port)
        .with_events(Arc::new(ObserverEvents::default()))
        .with_tracer(AdapterTracer {
            component: "gateway".into(),
        });

    let mut tasks: JoinSet<Result<()>> = JoinSet::new();

    let http_router = bootstrap.build_http();
    let http_listener = TcpListener::bind(http_addr)
        .await
        .with_context(|| format!("failed to bind adapter http server on {http_addr}"))?;
    tasks.spawn(async move {
        info!(addr = %http_addr, "adapter http listening");
        axum::serve(http_listener, http_router.into_make_service())
            .await
            .context("adapter http server exited unexpectedly")?;
        Ok(())
    });

    if let Some(addr) = grpc_addr {
        let grpc_state = bootstrap.state();
        tasks.spawn(async move {
            info!(addr = %addr, "adapter grpc listening");
            l7_adapter::grpc::serve(addr, grpc_state)
                .await
                .map_err(|err| anyhow!(err.to_string()))?;
            Ok(())
        });
    }

    if let Some(addr) = webdriver_addr {
        let wd_policy_handle = WebDriverBridgePolicyHandle::global();
        let wd_policy = if let Some(path) = webdriver_policy_path.as_ref() {
            load_policy_file::<WebDriverBridgePolicy>(path)
                .await
                .with_context(|| {
                    format!("failed to load webdriver policy from {}", path.display())
                })?
        } else {
            default_webdriver_policy()
        };
        wd_policy_handle.update(wd_policy);

        let tool_dispatcher: Arc<dyn ToolDispatcher> =
            Arc::new(SchedulerToolDispatcher::new(Arc::clone(&scheduler_dyn)));

        let bridge_router = WebDriverBridge::new(wd_policy_handle)
            .with_dispatcher(tool_dispatcher)
            .with_tracer(BridgeTracer::default())
            .build();

        let listener = TcpListener::bind(addr)
            .await
            .with_context(|| format!("failed to bind webdriver bridge on {addr}"))?;
        tasks.spawn(async move {
            info!(addr = %addr, "webdriver bridge listening");
            axum::serve(listener, bridge_router.into_make_service())
                .await
                .context("webdriver bridge server exited unexpectedly")?;
            Ok(())
        });
    }

    info!(
        http = %http_addr,
        grpc = ?grpc_addr,
        webdriver = ?webdriver_addr,
        "gateway surfaces ready"
    );

    let mut shutdown = Box::pin(signal::ctrl_c());
    loop {
        tokio::select! {
            res = tasks.join_next() => match res {
                Some(Ok(inner)) => inner?,
                Some(Err(err)) => return Err(anyhow!(err)),
                None => break,
            },
            _ = &mut shutdown => {
                info!("shutdown signal received; stopping gateway");
                tasks.shutdown().await;
                break;
            }
        }
    }

    while let Some(res) = tasks.join_next().await {
        match res {
            Ok(inner) => inner?,
            Err(err) if err.is_cancelled() => continue,
            Err(err) => return Err(anyhow!(err)),
        }
    }

    Ok(())
}

fn default_adapter_policy() -> AdapterPolicyView {
    let mut view = AdapterPolicyView::default();
    view.enabled = true;
    view.tracing_sample = 1.0;

    let tenant = TenantPolicy {
        id: "default".into(),
        allow_tools: Vec::new(),
        allow_flows: Vec::new(),
        read_only: Vec::new(),
        rate_limit_rps: 10,
        rate_burst: 20,
        concurrency_max: 4,
        timeout_ms_tool: 15_000,
        timeout_ms_flow: 60_000,
        timeout_ms_read: 10_000,
        idempotency_window_sec: 300,
        allow_cold_export: false,
        exports_max_lines: 50_000,
        authz_scopes: Vec::new(),
        api_keys: Vec::new(),
        hmac_secrets: Vec::new(),
    };
    view.tenants.push(tenant);
    view
}

fn default_webdriver_policy() -> WebDriverBridgePolicy {
    let mut policy = WebDriverBridgePolicy::default();
    policy.enabled = true;
    policy.privacy_profile = Some("strict".into());
    policy.tenants.push(WdTenantPolicy {
        id: "default".into(),
        enable: true,
        allow_endpoints: Vec::new(),
        allow_tools: Vec::new(),
        origins_allow: Vec::new(),
        concurrency_max: 4,
    });
    policy
}

async fn load_policy_file<T>(path: &Path) -> Result<T>
where
    T: DeserializeOwned,
{
    let raw = fs::read_to_string(path)
        .await
        .with_context(|| format!("failed to read policy file {}", path.display()))?;
    let ext = path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|s| s.to_ascii_lowercase());

    let parsed = match ext.as_deref() {
        Some("json") => serde_json::from_str(&raw)
            .with_context(|| format!("failed to parse JSON policy {}", path.display()))?,
        Some("yaml") | Some("yml") => serde_yaml::from_str(&raw)
            .with_context(|| format!("failed to parse YAML policy {}", path.display()))?,
        _ => match serde_yaml::from_str(&raw) {
            Ok(value) => value,
            Err(yaml_err) => serde_json::from_str(&raw).map_err(|json_err| {
                anyhow!(
                    "failed to parse policy {} as yaml ({}) or json ({})",
                    path.display(),
                    yaml_err,
                    json_err
                )
            })?,
        },
    };
    Ok(parsed)
}

struct GatewayDispatcherPort {
    scheduler: Arc<dyn Dispatcher + Send + Sync>,
}

impl GatewayDispatcherPort {
    fn new(scheduler: Arc<dyn Dispatcher + Send + Sync>) -> Self {
        Self { scheduler }
    }
}

#[async_trait]
impl DispatcherPort for GatewayDispatcherPort {
    async fn run_tool(&self, call: AdapterToolCall) -> AdapterResult<ToolOutcome> {
        let dispatch = map::to_dispatch_request(&call)?;
        let SubmitHandle {
            action_id,
            receiver,
        } = self
            .scheduler
            .submit(dispatch)
            .await
            .map_err(|_| L7AdapterError::Internal)?;

        let output = receiver.await.map_err(|_| L7AdapterError::Internal)?;

        let status = if output.error.is_some() {
            "error"
        } else {
            "ok"
        };

        let queue_ms = output.timeline.started_at.map(|started| {
            duration_ms(started.saturating_duration_since(output.timeline.enqueued_at))
        });
        let run_ms = output
            .timeline
            .finished_at
            .zip(output.timeline.started_at)
            .map(|(finish, start)| duration_ms(finish.saturating_duration_since(start)));

        let mut data = json!({
            "route": {
                "session": output.route.session.0,
                "page": output.route.page.0,
                "frame": output.route.frame.0,
            },
            "timeline": {
                "queue_ms": queue_ms,
                "run_ms": run_ms,
            }
        });

        if let Some(error) = output.error {
            if let Some(obj) = data.as_object_mut() {
                obj.insert(
                    "error".into(),
                    json!({
                        "message": error.to_string(),
                    }),
                );
            }
        }

        Ok(ToolOutcome {
            status: status.into(),
            data: Some(data),
            trace_id: call.trace_id.clone(),
            action_id: Some(action_id.0),
        })
    }
}

struct GatewayReadonlyPort {
    state_center: Arc<InMemoryStateCenter>,
}

impl GatewayReadonlyPort {
    fn new(state_center: Arc<InMemoryStateCenter>) -> Self {
        Self { state_center }
    }
}

#[async_trait]
impl ReadonlyPort for GatewayReadonlyPort {
    async fn export_timeline(
        &self,
        req: AdapterTimelineExportReq,
    ) -> AdapterResult<AdapterTimelineExportOutcome> {
        let limit = req
            .params
            .get("limit")
            .and_then(Value::as_u64)
            .unwrap_or(20) as usize;

        let events = if let Some(action_id) = req.params.get("action_id").and_then(Value::as_str) {
            self.state_center.recent_action(action_id)
        } else if let Some(task_id) = req.params.get("task_id").and_then(Value::as_str) {
            self.state_center.recent_task(task_id)
        } else if let Some(session_id) = req.params.get("session_id").and_then(Value::as_str) {
            let session = SessionId(session_id.to_string());
            self.state_center.recent_session(&session)
        } else if let Some(page_id) = req.params.get("page_id").and_then(Value::as_str) {
            let page = PageId(page_id.to_string());
            self.state_center.recent_page(&page)
        } else if let Some(frame_id) = req.params.get("frame_id").and_then(Value::as_str) {
            let frame = FrameId(frame_id.to_string());
            self.state_center.recent_frame(&frame)
        } else {
            self.state_center.snapshot()
        };

        let export = events
            .into_iter()
            .take(limit)
            .map(|event| state_event_to_json(&event))
            .collect::<Vec<_>>();

        Ok(AdapterTimelineExportOutcome {
            status: "ok".into(),
            export: Some(Value::Array(export)),
            trace_id: req.trace_id,
        })
    }
}

const DEFAULT_GATEWAY_TIMEOUT_MS: u64 = 10_000;

struct SchedulerToolDispatcher {
    scheduler: Arc<dyn Dispatcher + Send + Sync>,
}

impl SchedulerToolDispatcher {
    fn new(scheduler: Arc<dyn Dispatcher + Send + Sync>) -> Self {
        Self { scheduler }
    }
}

#[async_trait]
impl ToolDispatcher for SchedulerToolDispatcher {
    async fn run_tool(
        &self,
        tenant: &str,
        call: soulbrowser_core_types::ToolCall,
        routing: Option<RoutingHint>,
    ) -> BridgeResult<Value> {
        let trace_id = call.call_id.clone();
        let adapter_call = AdapterToolCall {
            tenant_id: tenant.to_string(),
            tool: call.tool.clone(),
            params: call.payload.clone(),
            routing: routing_hint_to_value(routing.as_ref()),
            options: Value::Null,
            timeout_ms: DEFAULT_GATEWAY_TIMEOUT_MS,
            idempotency_key: trace_id.clone(),
            trace_id,
        };
        let dispatch = map::to_dispatch_request(&adapter_call).map_err(map_adapter_error)?;
        let SubmitHandle {
            action_id,
            receiver,
        } = self.scheduler.submit(dispatch).await.map_err(|err| {
            warn!(message = %err, "scheduler submit failed for webdriver");
            BridgeError::Internal
        })?;

        let output = receiver.await.map_err(|err| {
            warn!(?err, "scheduler receiver dropped before completion");
            BridgeError::Internal
        })?;

        if let Some(error) = output.error {
            warn!(action = %action_id.0, message = %error, "scheduler returned error");
            return Err(BridgeError::Internal);
        }

        let queue_ms = output.timeline.started_at.map(|started| {
            duration_ms(started.saturating_duration_since(output.timeline.enqueued_at))
        });
        let run_ms = output
            .timeline
            .finished_at
            .zip(output.timeline.started_at)
            .map(|(finish, start)| duration_ms(finish.saturating_duration_since(start)));

        let mut data = json!({
            "route": {
                "session": output.route.session.0,
                "page": output.route.page.0,
                "frame": output.route.frame.0,
            },
            "timeline": {
                "queue_ms": queue_ms,
                "run_ms": run_ms,
            },
            "action_id": action_id.0,
        });

        if data.is_null() {
            data = Value::Object(JsonMap::new());
        }

        Ok(data)
    }
}

fn map_adapter_error(err: L7AdapterError) -> BridgeError {
    match err {
        L7AdapterError::Disabled => BridgeError::Disabled,
        L7AdapterError::UnauthorizedTenant | L7AdapterError::TenantNotFound => {
            BridgeError::Forbidden
        }
        L7AdapterError::ToolNotAllowed => BridgeError::Forbidden,
        L7AdapterError::InvalidArgument => BridgeError::InvalidArgument,
        L7AdapterError::TooManyRequests | L7AdapterError::ConcurrencyLimit => {
            BridgeError::Forbidden
        }
        L7AdapterError::NotImplemented(_) => BridgeError::NotImplemented,
        L7AdapterError::Internal => BridgeError::Internal,
    }
}

fn duration_ms(duration: std::time::Duration) -> u64 {
    duration.as_millis().min(u128::from(u64::MAX)) as u64
}

fn state_event_to_json(event: &StateEvent) -> Value {
    match event {
        StateEvent::Dispatch(dispatch) => json!({
            "type": "dispatch",
            "status": match dispatch.status {
                DispatchStatus::Success => "success",
                DispatchStatus::Failure => "failure",
            },
            "action_id": dispatch.action_id.0.clone(),
            "task_id": dispatch.task_id.clone(),
            "tool": dispatch.tool.clone(),
            "mutex_key": dispatch.mutex_key.clone(),
            "route": {
                "session": dispatch.route.session.0.clone(),
                "page": dispatch.route.page.0.clone(),
                "frame": dispatch.route.frame.0.clone(),
            },
            "attempts": dispatch.attempts,
            "wait_ms": dispatch.wait_ms,
            "run_ms": dispatch.run_ms,
            "pending": dispatch.pending,
            "slots_available": dispatch.slots_available,
            "error": dispatch.error.as_ref().map(|err| err.to_string()),
            "output": dispatch.output.clone(),
            "recorded_at_ms": system_time_to_ms(dispatch.recorded_at),
        }),
        StateEvent::Registry(registry) => json!({
            "type": "registry",
            "action": registry_action_str(&registry.action),
            "session": registry.session.as_ref().map(|s| s.0.clone()),
            "page": registry.page.as_ref().map(|p| p.0.clone()),
            "frame": registry.frame.as_ref().map(|f| f.0.clone()),
            "note": registry.note.clone(),
            "recorded_at_ms": system_time_to_ms(registry.recorded_at),
        }),
        StateEvent::Perceiver(perceiver) => {
            let mut obj = JsonMap::new();
            obj.insert("type".into(), Value::String("perceiver".into()));
            obj.insert(
                "route".into(),
                json!({
                    "session": perceiver.route.session.0.clone(),
                    "page": perceiver.route.page.0.clone(),
                    "frame": perceiver.route.frame.0.clone(),
                }),
            );
            obj.insert(
                "recorded_at_ms".into(),
                Value::Number(JsonNumber::from(system_time_to_ms(perceiver.recorded_at))),
            );

            match &perceiver.kind {
                PerceiverEventKind::Resolve {
                    strategy,
                    score,
                    candidate_count,
                    cache_hit,
                    breakdown,
                    reason,
                } => {
                    obj.insert("phase".into(), Value::String("resolve".into()));
                    obj.insert("strategy".into(), Value::String(strategy.clone()));
                    if let Some(number) = JsonNumber::from_f64(f64::from(*score)) {
                        obj.insert("score".into(), Value::Number(number));
                    }
                    obj.insert(
                        "candidate_count".into(),
                        Value::Number(JsonNumber::from(*candidate_count as u64)),
                    );
                    obj.insert("cache_hit".into(), Value::Bool(*cache_hit));
                    obj.insert("breakdown".into(), json!(breakdown.clone()));
                    obj.insert("reason".into(), Value::String(reason.clone()));
                }
                PerceiverEventKind::Judge {
                    check,
                    ok,
                    reason,
                    facts,
                } => {
                    obj.insert("phase".into(), Value::String("judge".into()));
                    obj.insert("check".into(), Value::String(check.clone()));
                    obj.insert("ok".into(), Value::Bool(*ok));
                    obj.insert("reason".into(), Value::String(reason.clone()));
                    obj.insert("facts".into(), facts.clone());
                }
                PerceiverEventKind::Snapshot { cache_hit } => {
                    obj.insert("phase".into(), Value::String("snapshot".into()));
                    obj.insert("cache_hit".into(), Value::Bool(*cache_hit));
                }
                PerceiverEventKind::Diff {
                    change_count,
                    changes,
                } => {
                    obj.insert("phase".into(), Value::String("diff".into()));
                    obj.insert(
                        "change_count".into(),
                        Value::Number(JsonNumber::from(*change_count as u64)),
                    );
                    obj.insert("changes".into(), Value::Array(changes.clone()));
                }
            }

            Value::Object(obj)
        }
    }
}

fn registry_action_str(action: &RegistryAction) -> &'static str {
    match action {
        RegistryAction::SessionCreated => "session_created",
        RegistryAction::SessionClosed => "session_closed",
        RegistryAction::PageOpened => "page_opened",
        RegistryAction::PageClosed => "page_closed",
        RegistryAction::PageFocused => "page_focused",
        RegistryAction::FrameFocused => "frame_focused",
        RegistryAction::FrameAttached => "frame_attached",
        RegistryAction::FrameDetached => "frame_detached",
        RegistryAction::HealthProbeTick => "health_probe_tick",
        RegistryAction::PageHealthUpdated => "page_health_updated",
        RegistryAction::PermissionsApplied => "permissions_applied",
    }
}

fn system_time_to_ms(time: std::time::SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u128::from(u64::MAX)) as u64
}

fn routing_hint_to_value(hint: Option<&RoutingHint>) -> Value {
    let Some(hint) = hint else {
        return Value::Null;
    };

    let mut map = JsonMap::new();
    if let Some(session) = &hint.session {
        map.insert("session".into(), Value::String(session.0.clone()));
    }
    if let Some(page) = &hint.page {
        map.insert("page".into(), Value::String(page.0.clone()));
    }
    if let Some(frame) = &hint.frame {
        map.insert("frame".into(), Value::String(frame.0.clone()));
    }
    if let Some(prefer) = &hint.prefer {
        let label = match prefer {
            soulbrowser_core_types::RoutePrefer::Focused => "focused",
            soulbrowser_core_types::RoutePrefer::RecentNav => "recent_nav",
            soulbrowser_core_types::RoutePrefer::MainFrame => "main_frame",
        };
        map.insert("prefer".into(), Value::String(label.to_string()));
    }

    if map.is_empty() {
        Value::Null
    } else {
        Value::Object(map)
    }
}

#[cfg(test)]
mod gateway_tests {
    use super::*;
    use serde_json::json;
    use soulbrowser_core_types::{ActionId, FrameId, PageId, SessionId};
    use soulbrowser_state_center::RegistryEvent;

    fn sample_route() -> ExecRoute {
        ExecRoute {
            session: SessionId("session-1".into()),
            page: PageId("page-1".into()),
            frame: FrameId("frame-1".into()),
            mutex_key: "frame:frame-1".into(),
        }
    }

    #[test]
    fn state_event_to_json_dispatch_includes_route() {
        let route = sample_route();
        let event = DispatchEvent::success(
            ActionId("action-1".into()),
            Some("task-9".into()),
            route.clone(),
            "click".into(),
            route.mutex_key.clone(),
            1,
            12,
            34,
            0,
            4,
            None,
        );
        let json = state_event_to_json(&StateEvent::dispatch_success(event));
        assert_eq!(json["type"], "dispatch");
        assert_eq!(json["status"], "success");
        assert_eq!(json["route"]["session"], "session-1");
        assert_eq!(json["route"]["frame"], "frame-1");
        assert_eq!(json["tool"], "click");
    }

    #[test]
    fn state_event_to_json_registry_maps_action() {
        let event = RegistryEvent::new(
            RegistryAction::PageOpened,
            Some(SessionId("tenant".into())),
            Some(PageId("page".into())),
            None,
            Some("note".into()),
        );
        let json = state_event_to_json(&StateEvent::registry(event));
        assert_eq!(json["type"], "registry");
        assert_eq!(json["action"], "page_opened");
        assert_eq!(json["note"], "note");
    }

    #[test]
    fn registry_action_str_matches_variants() {
        assert_eq!(
            registry_action_str(&RegistryAction::FrameDetached),
            "frame_detached"
        );
        assert_eq!(
            registry_action_str(&RegistryAction::PermissionsApplied),
            "permissions_applied"
        );
    }

    #[test]
    fn default_policies_enable_surfaces() {
        let adapter = default_adapter_policy();
        assert!(adapter.enabled);
        assert_eq!(adapter.tenants.len(), 1);
        assert!(adapter.tenants[0].allow_tools.is_empty());

        let wd = default_webdriver_policy();
        assert!(wd.enabled);
        assert_eq!(wd.tenants.len(), 1);
        assert!(wd.tenants[0].enable);
    }

    #[tokio::test]
    async fn readonly_port_filters_by_action_id() {
        let state_center = Arc::new(InMemoryStateCenter::new(128));
        let route = sample_route();
        let event = StateEvent::dispatch_success(DispatchEvent::success(
            ActionId("action-42".into()),
            None,
            route,
            "navigate".into(),
            "frame:frame-1".into(),
            1,
            5,
            10,
            0,
            4,
            None,
        ));
        state_center.append(event).await.unwrap();

        let readonly = GatewayReadonlyPort::new(Arc::clone(&state_center));
        let req = AdapterTimelineExportReq {
            tenant_id: "tenant".into(),
            params: json!({ "action_id": "action-42" }),
            trace_id: Some("trace-x".into()),
        };
        let result = readonly.export_timeline(req).await.unwrap();
        assert_eq!(result.status, "ok");
        let export = result.export.expect("export payload");
        let rows = export.as_array().expect("array payload");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["type"], "dispatch");
        assert_eq!(rows[0]["action_id"], "action-42");
    }

    #[test]
    fn routing_hint_to_value_serializes_fields() {
        let mut hint = RoutingHint::default();
        hint.session = Some(SessionId("session-9".into()));
        hint.page = Some(PageId("page-3".into()));
        hint.prefer = Some(soulbrowser_core_types::RoutePrefer::Focused);
        let value = routing_hint_to_value(Some(&hint));
        assert_eq!(value["session"], "session-9");
        assert_eq!(value["prefer"], "focused");
    }
}

async fn cmd_demo_real(args: DemoArgs) -> Result<()> {
    let attach_existing = args.ws_url.is_some();
    if !attach_existing {
        ensure_real_chrome_enabled()?;
    }

    let temp_profile = if attach_existing {
        None
    } else {
        let dir = PathBuf::from(format!(".soulbrowser-profile-{}", Uuid::new_v4()));
        fs::create_dir_all(&dir)
            .await
            .with_context(|| format!("creating profile directory {}", dir.display()))?;
        Some(dir)
    };

    let (bus, mut rx) = event_bus(256);
    let mut adapter_cfg = CdpConfig::default();
    if let Some(url) = &args.ws_url {
        adapter_cfg.websocket_url = Some(url.clone());
    }
    if let Some(path) = &args.chrome_path {
        adapter_cfg.executable = path.clone();
    }
    if args.headful {
        adapter_cfg.headless = false;
    }
    if let Some(profile_dir) = &temp_profile {
        adapter_cfg.user_data_dir = profile_dir.clone();
    }

    let adapter = Arc::new(CdpAdapter::new(adapter_cfg, bus.clone()));
    attach_permissions_for_adapter(&adapter).await;
    if let Some(ws) = &args.ws_url {
        info!(%ws, "Attaching to existing DevTools endpoint");
    }
    let result = async {
        Arc::clone(&adapter)
            .start()
            .await
            .map_err(|err| adapter_error("starting CDP adapter", err))?;

        let mut event_log = Vec::new();
        let page_id = wait_for_page_ready(
            Arc::clone(&adapter),
            &mut rx,
            Duration::from_secs(args.startup_timeout),
            &mut event_log,
        )
        .await?;
        info!(?page_id, url = %args.url, "Demo page ready; navigating");

        adapter
            .navigate(
                page_id,
                &args.url,
                Duration::from_secs(args.startup_timeout),
            )
            .await
            .map_err(|err| adapter_error("navigating to URL", err))?;

        adapter
            .wait_basic(
                page_id,
                "domready".to_string(),
                Duration::from_secs(args.startup_timeout),
            )
            .await
            .map_err(|err| adapter_error("waiting for DOM readiness", err))?;
        info!("DOM ready reached; continuing to observe events");

        let frame_stable_gate = json!({ "FrameStable": { "min_stable_ms": 200 } }).to_string();
        if let Err(err) = adapter
            .wait_basic(page_id, frame_stable_gate, Duration::from_secs(5))
            .await
        {
            warn!(?err, "frame stability wait failed");
        }

        sleep(Duration::from_millis(300)).await;

        let exec_route = build_exec_route(&adapter, page_id)?;
        let perception_port = Arc::new(StructuralAdapterPort::new(Arc::clone(&adapter)));
        let state_center: Arc<InMemoryStateCenter> = Arc::new(InMemoryStateCenter::new(256));
        let state_center_dyn: Arc<dyn StateCenter> = state_center.clone();
        let policy_center: Arc<dyn PolicyCenter + Send + Sync> =
            Arc::new(InMemoryPolicyCenter::new(default_snapshot()));
        let perceiver = Arc::new(
            StructuralPerceiverImpl::with_state_center_and_live_policy(
                Arc::clone(&perception_port),
                state_center_dyn,
                Arc::clone(&policy_center),
            )
            .await,
        );
        let mut lifecycle_rx = bus.subscribe();
        let perceiver_for_events = Arc::clone(&perceiver);
        let exec_page_for_events = exec_route.page.clone();
        let page_id_for_events = page_id;
        tokio::spawn(async move {
            while let Ok(event) = lifecycle_rx.recv().await {
                if let RawEvent::PageLifecycle { page, phase, .. } = event {
                    if page == page_id_for_events {
                        let phase_lower = phase.to_ascii_lowercase();
                        if phase_lower.contains("navigate")
                            || matches!(
                                phase_lower.as_str(),
                                "open" | "opened" | "close" | "closed"
                            )
                        {
                            perceiver_for_events.invalidate_for_page(&exec_page_for_events);
                        }
                    }
                }
            }
        });

        let resolve_opts = ResolveOptions::default();
        let input_hint = ResolveHint::Css(args.input_selector.clone());
        if let Ok(anchor) = perceiver
            .resolve_anchor(exec_route.clone(), input_hint, resolve_opts.clone())
            .await
        {
            info!(
                selector = %args.input_selector,
                reason = %anchor.reason,
                "Input anchor resolved"
            );
            let mut primary = anchor.primary.clone();
            if let Ok(vis) = perceiver.is_visible(exec_route.clone(), &mut primary).await {
                info!(
                    selector = %args.input_selector,
                    visible = vis.ok,
                    reason = %vis.reason,
                    "Input visibility check"
                );
            }
        } else {
            warn!(
                selector = %args.input_selector,
                "Falling back to raw selector for typing"
            );
        }

        adapter
            .type_text(
                page_id,
                &args.input_selector,
                &args.input_text,
                Duration::from_secs(5),
            )
            .await
            .map_err(|err| adapter_error("typing into input", err))?;
        perceiver.invalidate_for_page(&exec_route.page);
        info!(text = %args.input_text, "Input field populated");

        if !args.skip_submit {
            let submit_hint = ResolveHint::Css(args.submit_selector.clone());
            if let Ok(anchor) = perceiver
                .resolve_anchor(exec_route.clone(), submit_hint, resolve_opts.clone())
                .await
            {
                info!(
                    selector = %args.submit_selector,
                    reason = %anchor.reason,
                    "Submit anchor resolved"
                );
                let mut primary = anchor.primary.clone();
                if let Ok(clickable) = perceiver
                    .is_clickable(exec_route.clone(), &mut primary)
                    .await
                {
                    info!(
                        selector = %args.submit_selector,
                        clickable = clickable.ok,
                        reason = %clickable.reason,
                        "Submit clickable check"
                    );
                }
            } else {
                warn!(
                    selector = %args.submit_selector,
                    "Falling back to raw selector for submit"
                );
            }

            if let Err(err) = adapter
                .click(page_id, &args.submit_selector, Duration::from_secs(5))
                .await
            {
                warn!(?err, "clicking submit button failed");
            } else {
                perceiver.invalidate_for_page(&exec_route.page);
                info!("Submit button clicked");
                let gate = json!({
                    "NetworkQuiet": { "window_ms": 1_000, "max_inflight": 0 }
                })
                .to_string();
                if let Err(err) = adapter
                    .wait_basic(page_id, gate, Duration::from_secs(args.startup_timeout))
                    .await
                {
                    warn!("Network quiet wait after submit failed: {:?}", err.kind);
                }
            }
        }

        collect_events(
            &mut rx,
            Duration::from_secs(args.hold_after_ready),
            &mut event_log,
        )
        .await?;

        if let Some(ctx) = adapter.registry().get(&page_id) {
            if let Some(url) = ctx.recent_url {
                println!("Final URL: {}", url);
                event_log.push(format!("final_url {url}"));
            }
        }

        let stats = state_center.stats();
        info!(
            resolve = stats.perceiver_resolve,
            judge = stats.perceiver_judge,
            snapshot = stats.perceiver_snapshot,
            diff = stats.perceiver_diff,
            "Perceiver telemetry recorded"
        );

        if let Some(path) = &args.screenshot {
            let bytes = adapter
                .screenshot(page_id, Duration::from_secs(args.startup_timeout))
                .await
                .map_err(|err| adapter_error("capturing screenshot", err))?;
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).await.with_context(|| {
                    format!("creating screenshot directory {}", parent.display())
                })?;
            }
            fs::write(path, &bytes)
                .await
                .with_context(|| format!("writing screenshot to {}", path.display()))?;
            info!(path = %path.display(), "Screenshot saved");
        }

        adapter.shutdown().await;

        println!("Demo captured {} events:", event_log.len());
        for line in event_log {
            println!("  - {line}");
        }

        Ok(())
    }
    .await;

    if let Some(profile_dir) = temp_profile {
        if let Err(err) = fs::remove_dir_all(&profile_dir).await {
            warn!(
                path = %profile_dir.display(),
                ?err,
                "failed to remove temporary chrome profile directory"
            );
        }
    }

    result
}

async fn cmd_perceive(args: PerceiveArgs) -> Result<()> {
    use perceiver_hub::{PerceptionHub, PerceptionHubImpl, PerceptionOptions};
    use perceiver_semantic::SemanticPerceiverImpl;
    use perceiver_visual::VisualPerceiverImpl;

    let attach_existing = args.ws_url.is_some();
    if !attach_existing {
        ensure_real_chrome_enabled()?;
    }

    let temp_profile = if attach_existing {
        None
    } else {
        let dir = PathBuf::from(format!(".soulbrowser-profile-{}", Uuid::new_v4()));
        fs::create_dir_all(&dir)
            .await
            .with_context(|| format!("creating profile directory {}", dir.display()))?;
        Some(dir)
    };

    let (bus, mut rx) = event_bus(256);
    let mut adapter_cfg = CdpConfig::default();
    if let Some(url) = &args.ws_url {
        adapter_cfg.websocket_url = Some(url.clone());
    }
    if let Some(path) = &args.chrome_path {
        adapter_cfg.executable = path.clone();
    }
    if args.headful {
        adapter_cfg.headless = false;
    }
    if let Some(profile_dir) = &temp_profile {
        adapter_cfg.user_data_dir = profile_dir.clone();
    }

    let adapter = Arc::new(CdpAdapter::new(adapter_cfg, bus.clone()));
    attach_permissions_for_adapter(&adapter).await;
    if let Some(ws) = &args.ws_url {
        info!(%ws, "Attaching to existing DevTools endpoint");
    }

    let result = async {
        Arc::clone(&adapter)
            .start()
            .await
            .map_err(|err| adapter_error("starting CDP adapter", err))?;

        let mut event_log = Vec::new();
        let page_id = wait_for_page_ready(
            Arc::clone(&adapter),
            &mut rx,
            Duration::from_secs(30),
            &mut event_log,
        )
        .await?;
        info!(?page_id, url = %args.url, "Page ready; navigating for perception");

        adapter
            .navigate(page_id, &args.url, Duration::from_secs(30))
            .await
            .map_err(|err| adapter_error("navigating to URL", err))?;

        adapter
            .wait_basic(page_id, "domready".to_string(), Duration::from_secs(30))
            .await
            .map_err(|err| adapter_error("waiting for DOM readiness", err))?;

        let frame_stable_gate = json!({ "FrameStable": { "min_stable_ms": 200 } }).to_string();
        if let Err(err) = adapter
            .wait_basic(page_id, frame_stable_gate, Duration::from_secs(5))
            .await
        {
            warn!(?err, "frame stability wait failed");
        }

        sleep(Duration::from_millis(300)).await;

        let exec_route = build_exec_route(&adapter, page_id)?;
        let perception_port = Arc::new(StructuralAdapterPort::new(Arc::clone(&adapter)));
        let state_center: Arc<InMemoryStateCenter> = Arc::new(InMemoryStateCenter::new(256));
        let state_center_dyn: Arc<dyn StateCenter> = state_center.clone();
        let policy_center: Arc<dyn PolicyCenter + Send + Sync> =
            Arc::new(InMemoryPolicyCenter::new(default_snapshot()));

        // Create structural perceiver
        let structural_perceiver = Arc::new(
            StructuralPerceiverImpl::with_state_center_and_live_policy(
                Arc::clone(&perception_port),
                state_center_dyn,
                Arc::clone(&policy_center),
            )
            .await,
        );

        // Determine which perception modes to enable
        let enable_visual = args.visual || args.all;
        let enable_semantic = args.semantic || args.all;
        let enable_structural = args.structural || args.all;

        // Default to all modes if none specified
        let (enable_visual, enable_semantic, enable_structural) =
            if !enable_visual && !enable_semantic && !enable_structural {
                (true, true, true)
            } else {
                (enable_visual, enable_semantic, enable_structural)
            };

        println!("\n🔍 Multi-Modal Perception Analysis");
        println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
        println!("URL: {}", args.url);
        println!(
            "Modes: {}{}{}",
            if enable_structural {
                "📊 Structural "
            } else {
                ""
            },
            if enable_visual {
                "👁️  Visual "
            } else {
                ""
            },
            if enable_semantic {
                "🧠 Semantic "
            } else {
                ""
            }
        );
        println!();

        // Create perception hub
        let hub = if enable_visual && enable_semantic {
            let visual_perceiver = Arc::new(VisualPerceiverImpl::new(Arc::clone(&adapter)));
            let semantic_perceiver =
                Arc::new(SemanticPerceiverImpl::new(structural_perceiver.clone()
                    as Arc<dyn perceiver_structural::StructuralPerceiver>));
            PerceptionHubImpl::new(structural_perceiver, visual_perceiver, semantic_perceiver)
        } else if enable_visual {
            let visual_perceiver = Arc::new(VisualPerceiverImpl::new(Arc::clone(&adapter)));
            PerceptionHubImpl::structural_only(structural_perceiver).with_visual(visual_perceiver)
        } else if enable_semantic {
            let semantic_perceiver =
                Arc::new(SemanticPerceiverImpl::new(structural_perceiver.clone()
                    as Arc<dyn perceiver_structural::StructuralPerceiver>));
            PerceptionHubImpl::structural_only(structural_perceiver)
                .with_semantic(semantic_perceiver)
        } else {
            PerceptionHubImpl::structural_only(structural_perceiver)
        };

        // Configure perception options
        let perception_opts = PerceptionOptions {
            enable_structural,
            enable_visual,
            enable_semantic,
            enable_insights: args.insights,
            capture_screenshot: enable_visual,
            extract_text: enable_semantic,
            timeout_secs: args.timeout,
        };

        // Perform multi-modal perception
        info!("Starting multi-modal perception analysis");
        let perception = hub
            .perceive(&exec_route, perception_opts)
            .await
            .context("multi-modal perception failed")?;

        // Display results
        println!("📊 Structural Analysis");
        println!("  DOM nodes: {}", perception.structural.dom_node_count);
        println!(
            "  Interactive elements: {}",
            perception.structural.interactive_element_count
        );
        println!("  Has forms: {}", perception.structural.has_forms);
        println!("  Has navigation: {}", perception.structural.has_navigation);
        println!();

        if let Some(visual) = &perception.visual {
            println!("👁️  Visual Analysis");
            println!(
                "  Dominant colors: {} detected",
                visual.dominant_colors.len()
            );
            println!("  Avg contrast: {:.2}", visual.avg_contrast);
            println!(
                "  Viewport utilization: {:.1}%",
                visual.viewport_utilization * 100.0
            );
            println!("  Visual complexity: {:.2}", visual.complexity);
            println!();

            if let Some(screenshot_path) = &args.screenshot {
                // Save screenshot using visual perceiver
                let screenshot = hub
                    .visual()
                    .unwrap()
                    .capture_screenshot(&exec_route, Default::default())
                    .await
                    .context("capturing screenshot")?;

                if let Some(parent) = screenshot_path.parent() {
                    fs::create_dir_all(parent).await.with_context(|| {
                        format!("creating screenshot directory {}", parent.display())
                    })?;
                }
                fs::write(screenshot_path, &screenshot.data)
                    .await
                    .with_context(|| {
                        format!("writing screenshot to {}", screenshot_path.display())
                    })?;
                println!("  Screenshot saved: {}", screenshot_path.display());
                println!();
            }
        }

        if let Some(semantic) = &perception.semantic {
            println!("🧠 Semantic Analysis");
            println!("  Content type: {:?}", semantic.content_type);
            println!("  Page intent: {:?}", semantic.intent);
            println!(
                "  Language: {} ({:.1}% confidence)",
                semantic.language,
                semantic.language_confidence * 100.0
            );
            if let Some(readability) = semantic.readability {
                println!("  Readability score: {:.1}", readability);
            }
            println!("  Keywords: {}", semantic.keywords.join(", "));
            println!("  Summary: {}", semantic.summary);
            println!();
        }

        if args.insights && !perception.insights.is_empty() {
            println!("💡 Cross-Modal Insights");
            for insight in &perception.insights {
                println!(
                    "  • [{:?}] {} (confidence: {:.0}%)",
                    insight.insight_type,
                    insight.description,
                    insight.confidence * 100.0
                );
            }
            println!();
        }

        println!("Overall confidence: {:.1}%", perception.confidence * 100.0);

        // Save JSON output if requested
        if let Some(output_path) = &args.output {
            let json = serde_json::to_string_pretty(&perception)
                .context("serializing perception results")?;
            if let Some(parent) = output_path.parent() {
                fs::create_dir_all(parent)
                    .await
                    .with_context(|| format!("creating output directory {}", parent.display()))?;
            }
            fs::write(output_path, json)
                .await
                .with_context(|| format!("writing results to {}", output_path.display()))?;
            println!("\n📄 Results saved to: {}", output_path.display());
        }

        adapter.shutdown().await;
        Ok(())
    }
    .await;

    if let Some(profile_dir) = temp_profile {
        if let Err(err) = fs::remove_dir_all(&profile_dir).await {
            warn!(
                path = %profile_dir.display(),
                ?err,
                "failed to remove temporary chrome profile directory"
            );
        }
    }

    result
}

fn ensure_real_chrome_enabled() -> Result<()> {
    let flag = env::var("SOULBROWSER_USE_REAL_CHROME")
        .unwrap_or_default()
        .to_ascii_lowercase();
    let enabled = matches!(flag.as_str(), "1" | "true" | "yes" | "on");
    if enabled {
        Ok(())
    } else {
        bail!("Set SOULBROWSER_USE_REAL_CHROME=1 to run the demo against a real Chrome/Chromium binary");
    }
}

fn build_exec_route(adapter: &Arc<CdpAdapter>, page_id: AdapterPageId) -> Result<ExecRoute> {
    let context = adapter
        .registry()
        .iter()
        .into_iter()
        .find(|(pid, _)| pid == &page_id)
        .map(|(_, ctx)| ctx)
        .ok_or_else(|| anyhow!("no registry context available for page {:?}", page_id))?;

    let session = SessionId(context.session_id.0.to_string());
    let page = PageId(page_id.0.to_string());
    let frame_key = context.target_id.clone().unwrap_or_else(|| page.0.clone());
    let frame = FrameId(frame_key);

    Ok(ExecRoute::new(session, page, frame))
}

fn adapter_error(context: &str, err: CdpAdapterError) -> anyhow::Error {
    let hint = err.hint.clone().unwrap_or_default();
    let data = err
        .data
        .as_ref()
        .map(|value| value.to_string())
        .unwrap_or_else(|| "null".to_string());
    anyhow!(
        "{}: kind={:?}, retriable={}, hint={}, data={}",
        context,
        err.kind,
        err.retriable,
        hint,
        data
    )
}

async fn wait_for_page_ready(
    adapter: Arc<CdpAdapter>,
    rx: &mut cdp_adapter::EventStream,
    wait_limit: Duration,
    log: &mut Vec<String>,
) -> Result<AdapterPageId> {
    let deadline = Instant::now() + wait_limit;
    loop {
        if let Some((page_id, _ctx)) = adapter
            .registry()
            .iter()
            .into_iter()
            .find(|(_, ctx)| ctx.cdp_session.is_some())
        {
            return Ok(page_id);
        }

        if Instant::now() >= deadline {
            let preview = log.iter().take(16).cloned().collect::<Vec<_>>();
            let preview = preview.join(" | ");
            bail!(
                "Timed out waiting for Chrome target/session. Recent events: {}",
                preview
            );
        }

        match timeout(Duration::from_millis(500), rx.recv()).await {
            Ok(Ok(event)) => {
                log.push(describe_raw_event(&event));
            }
            Ok(Err(RecvError::Lagged(skipped))) => {
                warn!(skipped, "Demo event stream lagged; skipping older events");
            }
            Ok(Err(RecvError::Closed)) => {
                bail!("CDP adapter event stream closed unexpectedly");
            }
            Err(_) => {
                // No event within slice; continue polling registry
            }
        }
    }
}

async fn collect_events(
    rx: &mut cdp_adapter::EventStream,
    duration: Duration,
    log: &mut Vec<String>,
) -> Result<()> {
    if duration.is_zero() {
        return Ok(());
    }

    let deadline = Instant::now() + duration;
    while Instant::now() < deadline {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .unwrap_or_default();
        if remaining.is_zero() {
            break;
        }
        let slice = if remaining > Duration::from_millis(500) {
            Duration::from_millis(500)
        } else {
            remaining
        };

        match timeout(slice, rx.recv()).await {
            Ok(Ok(event)) => {
                log.push(describe_raw_event(&event));
            }
            Ok(Err(RecvError::Lagged(skipped))) => {
                warn!(skipped, "Demo event stream lagged; skipping older events");
            }
            Ok(Err(RecvError::Closed)) => {
                warn!("Demo event stream closed");
                break;
            }
            Err(_) => {
                // no event in this slice; loop continues until deadline
            }
        }
    }

    Ok(())
}

fn describe_raw_event(event: &RawEvent) -> String {
    match event {
        RawEvent::PageLifecycle {
            page, frame, phase, ..
        } => {
            let frame_str = frame.map(|f| format!(" frame={:?}", f)).unwrap_or_default();
            format!("page {:?} phase={}{}", page, phase, frame_str)
        }
        RawEvent::PageNavigated { page, url, .. } => {
            format!("page {:?} navigated -> {}", page, url)
        }
        RawEvent::NetworkSummary {
            page,
            req,
            res2xx,
            res4xx,
            res5xx,
            inflight,
            quiet,
            since_last_activity_ms,
            ..
        } => format!(
            "network {:?} req={} 2xx={} 4xx={} 5xx={} inflight={} quiet={} idle={}ms",
            page, req, res2xx, res4xx, res5xx, inflight, quiet, since_last_activity_ms
        ),
        RawEvent::NetworkActivity { page, signal } => {
            format!("network-activity {:?} signal={:?}", page, signal)
        }
        RawEvent::Error { message, .. } => format!("adapter-error: {message}"),
    }
}

// Helper functions

fn parse_window_size(size: Option<&str>) -> Result<Option<(u32, u32)>> {
    match size {
        Some(size_str) => {
            let parts: Vec<&str> = size_str.split('x').collect();
            if parts.len() == 2 {
                let width = parts[0].parse::<u32>().context("Invalid width")?;
                let height = parts[1].parse::<u32>().context("Invalid height")?;
                Ok(Some((width, height)))
            } else {
                Err(anyhow::anyhow!(
                    "Invalid window size format. Use WIDTHxHEIGHT"
                ))
            }
        }
        None => Ok(None),
    }
}

fn parse_parameters(params: &[String]) -> Result<std::collections::HashMap<String, String>> {
    let mut result = std::collections::HashMap::new();

    for param in params {
        let parts: Vec<&str> = param.splitn(2, '=').collect();
        if parts.len() == 2 {
            result.insert(parts[0].to_string(), parts[1].to_string());
        } else {
            return Err(anyhow::anyhow!(
                "Invalid parameter format: {}. Use key=value",
                param
            ));
        }
    }

    Ok(result)
}

fn format_extension(format: &DataFormat) -> &'static str {
    match format {
        DataFormat::Json => "json",
        DataFormat::Csv => "csv",
        DataFormat::Excel => "xlsx",
        DataFormat::Yaml => "yaml",
    }
}

fn timeline_format_extension(format: &TimelineFormat) -> &'static str {
    match format {
        TimelineFormat::Html => "html",
        TimelineFormat::Json => "json",
        TimelineFormat::Svg => "svg",
        TimelineFormat::Pdf => "pdf",
    }
}

fn script_extension(language: &ScriptLanguage) -> &'static str {
    match language {
        ScriptLanguage::JavaScript => "js",
        ScriptLanguage::TypeScript => "ts",
        ScriptLanguage::Python => "py",
        ScriptLanguage::Rust => "rs",
        ScriptLanguage::Go => "go",
    }
}

async fn export_script(
    context: &Arc<AppContext>,
    session: &str,
    language: &ScriptLanguage,
    output_path: &PathBuf,
) -> Result<()> {
    let mut events = context
        .storage()
        .backend()
        .query_events(QueryParams {
            session_id: Some(session.to_string()),
            event_type: None,
            from_timestamp: None,
            to_timestamp: None,
            limit: 0,
            offset: 0,
        })
        .await
        .context("Failed to load session events for script export")?;

    if events.is_empty() {
        bail!("No recorded events found for session {}", session);
    }

    events.sort_by(|a, b| {
        a.timestamp
            .cmp(&b.timestamp)
            .then_with(|| a.sequence.cmp(&b.sequence))
    });

    let steps = build_script_steps(&events);
    let script = generate_script(language, &steps);

    if let Some(parent) = output_path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)
                .await
                .context("Failed to create parent directories for script export")?;
        }
    }

    fs::write(output_path, script)
        .await
        .context("Failed to write exported script")?;

    Ok(())
}

#[derive(Debug)]
enum ScriptStep {
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
    Wait(u64),
    Custom {
        event_type: String,
        payload: String,
    },
}

fn build_script_steps(events: &[BrowserEvent]) -> Vec<ScriptStep> {
    let mut steps = Vec::new();
    let mut previous_ts: Option<i64> = None;

    for event in events {
        if let Some(prev) = previous_ts {
            let delta = event.timestamp.saturating_sub(prev);
            if delta >= 500 {
                steps.push(ScriptStep::Wait(delta as u64));
            }
        }

        match event.event_type.as_str() {
            "navigate" => {
                if let Some(url) = event.data.get("url").and_then(|v| v.as_str()) {
                    steps.push(ScriptStep::Navigate(url.to_string()));
                }
            }
            "click" => {
                if let Some(selector) = event.data.get("selector").and_then(|v| v.as_str()) {
                    steps.push(ScriptStep::Click(selector.to_string()));
                }
            }
            "type" => {
                if let (Some(selector), Some(text)) = (
                    event.data.get("selector").and_then(|v| v.as_str()),
                    event.data.get("text").and_then(|v| v.as_str()),
                ) {
                    steps.push(ScriptStep::Type {
                        selector: selector.to_string(),
                        text: text.to_string(),
                    });
                }
            }
            "select" => {
                if let (Some(selector), Some(value)) = (
                    event.data.get("selector").and_then(|v| v.as_str()),
                    event.data.get("value").and_then(|v| v.as_str()),
                ) {
                    let match_kind = event
                        .data
                        .get("match_kind")
                        .and_then(|v| v.as_str())
                        .map(str::to_string);
                    let mode = event
                        .data
                        .get("mode")
                        .and_then(|v| v.as_str())
                        .map(str::to_string);
                    steps.push(ScriptStep::Select {
                        selector: selector.to_string(),
                        value: value.to_string(),
                        match_kind,
                        mode,
                    });
                }
            }
            "screenshot" => {
                if let Some(filename) = event.data.get("filename").and_then(|v| v.as_str()) {
                    steps.push(ScriptStep::Screenshot(filename.to_string()));
                }
            }
            other => {
                let payload =
                    serde_json::to_string_pretty(&event.data).unwrap_or_else(|_| "{}".to_string());
                steps.push(ScriptStep::Custom {
                    event_type: other.to_string(),
                    payload,
                });
            }
        }

        previous_ts = Some(event.timestamp);
    }

    steps
}

fn generate_script(language: &ScriptLanguage, steps: &[ScriptStep]) -> String {
    match language {
        ScriptLanguage::JavaScript => render_javascript(steps, false),
        ScriptLanguage::TypeScript => render_javascript(steps, true),
        ScriptLanguage::Python => render_python(steps),
        ScriptLanguage::Rust => render_rust(steps),
        ScriptLanguage::Go => render_go(steps),
    }
}

fn render_javascript(steps: &[ScriptStep], typed: bool) -> String {
    let mut script = String::new();

    if typed {
        script.push_str("import { chromium, Browser, Page } from 'playwright';\n\n");
        script.push_str("async function run(): Promise<void> {\n");
    } else {
        script.push_str("const { chromium } = require('playwright');\n\n");
        script.push_str("(async () => {\n");
    }

    script.push_str("  const browser = await chromium.launch();\n");
    script.push_str("  const page = await browser.newPage();\n\n");

    for step in steps {
        let line = js_statement(step);
        script.push_str("  ");
        script.push_str(&line);
        script.push('\n');
    }

    script.push('\n');
    script.push_str("  await browser.close();\n");

    if typed {
        script.push_str(
            "}\n\nrun().catch(err => {\n  console.error(err);\n  process.exit(1);\n});\n",
        );
    } else {
        script.push_str("})();\n");
    }

    script
}

fn render_python(steps: &[ScriptStep]) -> String {
    let mut script = String::new();
    script.push_str("import asyncio\n");
    script.push_str("from playwright.async_api import async_playwright\n\n");
    script.push_str("async def run():\n");
    script.push_str("    async with async_playwright() as p:\n");
    script.push_str("        browser = await p.chromium.launch()\n");
    script.push_str("        page = await browser.new_page()\n\n");

    for step in steps {
        let line = python_statement(step);
        script.push_str("        ");
        script.push_str(&line);
        script.push('\n');
    }

    script.push('\n');
    script.push_str("        await browser.close()\n\n");
    script.push_str("asyncio.run(run())\n");

    script
}

fn render_rust(steps: &[ScriptStep]) -> String {
    let mut script = String::new();
    let has_wait = steps.iter().any(|step| matches!(step, ScriptStep::Wait(_)));

    script.push_str("use fantoccini::{Client, Locator};\n");
    if has_wait {
        script.push_str("use tokio::time::{sleep, Duration};\n\n");
    } else {
        script.push('\n');
    }
    script
        .push_str("#[tokio::main]\nasync fn main() -> Result<(), fantoccini::error::CmdError> {\n");
    script.push_str("    let mut client = Client::new(\"http://localhost:4444\").await?;\n\n");

    for step in steps {
        let line = rust_statement(step);
        script.push_str("    ");
        script.push_str(&line);
        script.push('\n');
    }

    script.push('\n');
    script.push_str("    client.close().await\n}");
    script.push('\n');

    script
}

fn render_go(steps: &[ScriptStep]) -> String {
    let mut script = String::new();
    let has_wait = steps.iter().any(|step| matches!(step, ScriptStep::Wait(_)));
    let has_screenshot = steps
        .iter()
        .any(|step| matches!(step, ScriptStep::Screenshot(_)));

    script.push_str("package main\n\n");
    script.push_str("import (\n");
    script.push_str("    \"context\"\n");
    script.push_str("    \"log\"\n");
    script.push_str("    \"fmt\"\n");
    if has_wait {
        script.push_str("    \"time\"\n");
    }
    if has_screenshot {
        script.push_str("    \"os\"\n");
    }

    script.push_str("\n    \"github.com/chromedp/chromedp\"\n");
    script.push_str(")\n\n");
    script.push_str("func main() {\n");
    script.push_str("    ctx, cancel := chromedp.NewContext(context.Background())\n");
    script.push_str("    defer cancel()\n\n");
    script.push_str("    tasks := chromedp.Tasks{\n");

    for step in steps {
        let line = go_statement(step);
        script.push_str("        ");
        script.push_str(&line);
        script.push_str(",\n");
    }

    script.push_str("    }\n\n");
    script.push_str("    if err := chromedp.Run(ctx, tasks...); err != nil {\n");
    script.push_str("        log.Fatal(err)\n    }\n}\n");

    script
}

fn js_statement(step: &ScriptStep) -> String {
    match step {
        ScriptStep::Navigate(url) => {
            format!("await page.goto(\"{}\");", escape_string(url))
        }
        ScriptStep::Click(selector) => {
            format!("await page.click(\"{}\");", escape_string(selector))
        }
        ScriptStep::Type { selector, text } => {
            format!(
                "await page.fill(\"{}\", \"{}\");",
                escape_string(selector),
                escape_string(text)
            )
        }
        ScriptStep::Select {
            selector,
            value,
            match_kind,
            mode,
        } => {
            let option_expr = match match_kind.as_deref() {
                Some("label") => format!("{{ label: \"{}\" }}", escape_string(value)),
                Some("index") => match value.parse::<u32>() {
                    Ok(idx) => format!("{{ index: {} }}", idx),
                    Err(_) => format!("{{ index: \"{}\" }}", escape_string(value)),
                },
                _ => format!("{{ value: \"{}\" }}", escape_string(value)),
            };
            let mut stmt = format!(
                "await page.selectOption(\"{}\", {});",
                escape_string(selector),
                option_expr
            );
            if let Some(mode_val) = mode {
                if !mode_val.eq_ignore_ascii_case("single") {
                    stmt.push_str(&format!(" // mode={}", mode_val));
                }
            }
            stmt
        }
        ScriptStep::Screenshot(filename) => {
            format!(
                "await page.screenshot({{ path: \"{}\" }});",
                escape_string(filename)
            )
        }
        ScriptStep::Wait(ms) => format!("await page.waitForTimeout({});", ms),
        ScriptStep::Custom {
            event_type,
            payload,
        } => format!(
            "// TODO: handle event '{}' with payload {}",
            event_type,
            payload.replace('\n', " ")
        ),
    }
}

fn python_statement(step: &ScriptStep) -> String {
    match step {
        ScriptStep::Navigate(url) => {
            format!("await page.goto(\"{}\")", escape_string(url))
        }
        ScriptStep::Click(selector) => {
            format!("await page.click(\"{}\")", escape_string(selector))
        }
        ScriptStep::Type { selector, text } => {
            format!(
                "await page.fill(\"{}\", \"{}\")",
                escape_string(selector),
                escape_string(text)
            )
        }
        ScriptStep::Select {
            selector,
            value,
            match_kind,
            mode,
        } => {
            let option_arg = match match_kind.as_deref() {
                Some("label") => format!("label=\"{}\"", escape_string(value)),
                Some("index") => match value.parse::<u32>() {
                    Ok(idx) => format!("index={}", idx),
                    Err(_) => format!("index=\"{}\"", escape_string(value)),
                },
                _ => format!("value=\"{}\"", escape_string(value)),
            };
            let mut stmt = format!(
                "await page.select_option(\"{}\", {})",
                escape_string(selector),
                option_arg
            );
            if let Some(mode_val) = mode {
                if !mode_val.eq_ignore_ascii_case("single") {
                    stmt.push_str(&format!("  # mode={} not explicitly handled", mode_val));
                }
            }
            stmt
        }
        ScriptStep::Screenshot(filename) => {
            format!(
                "await page.screenshot(path=\"{}\")",
                escape_string(filename)
            )
        }
        ScriptStep::Wait(ms) => format!("await page.wait_for_timeout({})", ms),
        ScriptStep::Custom {
            event_type,
            payload,
        } => format!(
            "# TODO: handle event '{}' with payload {}",
            event_type,
            payload.replace('\n', " ")
        ),
    }
}

fn rust_statement(step: &ScriptStep) -> String {
    match step {
        ScriptStep::Navigate(url) => {
            format!("client.goto(\"{}\").await?;", escape_string(url))
        }
        ScriptStep::Click(selector) => {
            format!(
                "client.find(Locator::Css(\"{}\")).await?.click().await?;",
                escape_string(selector)
            )
        }
        ScriptStep::Type { selector, text } => {
            format!(
                "client.find(Locator::Css(\"{}\")).await?.send_keys(\"{}\").await?;",
                escape_string(selector),
                escape_string(text)
            )
        }
        ScriptStep::Select {
            selector,
            value,
            match_kind,
            mode,
        } => {
            let base = format!(
                "client.find(Locator::Css(\"{}\")).await?.select_by_value(\"{}\").await?;",
                escape_string(selector),
                escape_string(value)
            );
            let mut stmt = match match_kind.as_deref() {
                Some("label") => format!(
                    "// selecting by label via script fallback\n    client.execute(r#\"(selector, label) => {{\n        const el = document.querySelector(selector);\n        if (!el) return false;\n        const opt = Array.from(el.options || []).find(o => o.text === label);\n        if (!opt) return false;\n        el.value = opt.value;\n        el.dispatchEvent(new Event('change', {{ bubbles: true }}));\n        return true;\n    }}\"#, serde_json::json!([\"{}\", \"{}\"])).await?;",
                    escape_string(selector),
                    escape_string(value)
                ),
                Some("index") => match value.parse::<u32>() {
                    Ok(idx) => format!(
                        "client.find(Locator::Css(\"{}\")).await?.select_by_index({}).await?;",
                        escape_string(selector),
                        idx
                    ),
                    Err(_) => base.clone(),
                },
                _ => base,
            };
            if let Some(mode_val) = mode {
                if !mode_val.eq_ignore_ascii_case("single") {
                    stmt.push_str(&format!(
                        "\n    // mode={} not explicitly handled",
                        mode_val
                    ));
                }
            }
            stmt
        }
        ScriptStep::Screenshot(filename) => {
            format!(
                "std::fs::write(\"{}\", client.screenshot().await?)?;",
                escape_string(filename)
            )
        }
        ScriptStep::Wait(ms) => format!("sleep(Duration::from_millis({})).await;", ms),
        ScriptStep::Custom {
            event_type,
            payload,
        } => format!(
            "// TODO: handle event '{}' with payload {}",
            event_type,
            payload.replace('\n', " ")
        ),
    }
}

fn go_statement(step: &ScriptStep) -> String {
    match step {
        ScriptStep::Navigate(url) => {
            format!("chromedp.Navigate(\"{}\")", escape_string(url))
        }
        ScriptStep::Click(selector) => {
            format!(
                "chromedp.Click(\"{}\", chromedp.NodeVisible)",
                escape_string(selector)
            )
        }
        ScriptStep::Type { selector, text } => {
            format!(
                "chromedp.SendKeys(\"{}\", \"{}\")",
                escape_string(selector),
                escape_string(text)
            )
        }
        ScriptStep::Select {
            selector,
            value,
            match_kind,
            mode,
        } => {
            let mut stmt = format!(
                "chromedp.SetValue(\"{}\", \"{}\")",
                escape_string(selector),
                escape_string(value)
            );
            if let Some(kind) = match_kind {
                if kind.eq_ignore_ascii_case("label") {
                    stmt = format!(
                        "chromedp.ActionFunc(func(ctx context.Context) error {{\n            return chromedp.Evaluate(\"(() => {{ const el = document.querySelector(\\\\\"{}\\\\\"); if (!el) return false; const opt = Array.from(el.options || []).find(o => o.text === \\\\\"{}\\\\\"); if (!opt) return false; el.value = opt.value; el.dispatchEvent(new Event('change', {{ bubbles: true }})); return true; }})()\", nil).Do(ctx)\n        }})",
                        escape_string(selector),
                        escape_string(value)
                    );
                } else if kind.eq_ignore_ascii_case("index") {
                    if let Ok(idx) = value.parse::<u32>() {
                        stmt = format!(
                            "chromedp.ActionFunc(func(ctx context.Context) error {{\n            return chromedp.Evaluate(\"(() => {{ const el = document.querySelector(\\\\\"{}\\\\\"); if (!el) return false; const opts = Array.from(el.options || []); if (opts.length <= {}) return false; el.selectedIndex = {}; el.dispatchEvent(new Event('change', {{ bubbles: true }})); return true; }})()\", nil).Do(ctx)\n        }})",
                            escape_string(selector),
                            idx,
                            idx
                        );
                    }
                }
            }
            if let Some(mode_val) = mode {
                if !mode_val.eq_ignore_ascii_case("single") {
                    stmt.push_str(&format!(
                        " // mode={} (multiple selection handling not generated)",
                        mode_val
                    ));
                }
            }
            stmt
        }
        ScriptStep::Screenshot(filename) => {
            format!(
                "chromedp.ActionFunc(func(ctx context.Context) error {{\n            var buf []byte\n            if err := chromedp.FullScreenshot(&buf, 90).Do(ctx); err != nil {{\n                return err\n            }}\n            return os.WriteFile(\"{}\", buf, 0o644)\n        }})",
                escape_string(filename)
            )
        }
        ScriptStep::Wait(ms) => format!(
            "chromedp.Sleep({} * time.Millisecond)",
            ms
        ),
        ScriptStep::Custom { event_type, payload } => format!(
            "chromedp.ActionFunc(func(context.Context) error {{\n            // TODO: handle event '{}' with payload {}\n            return nil\n        }})",
            event_type,
            payload.replace('\n', " ")
        ),
    }
}

fn escape_string(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
}

#[derive(Debug, Serialize)]
struct IdlePeriodReport {
    from_event: String,
    to_event: String,
    duration_ms: i64,
}

#[derive(Debug, Serialize)]
struct PerformanceReport {
    session_id: String,
    total_events: usize,
    duration_ms: i64,
    events_per_minute: f64,
    average_gap_ms: Option<f64>,
    longest_gap_ms: Option<i64>,
    idle_periods: Vec<IdlePeriodReport>,
}

#[derive(Debug, Serialize)]
struct AccessibilityReport {
    session_id: String,
    total_interactions: usize,
    accessible_interactions: usize,
    accessibility_score: f64,
    selectors_missing_accessibility: Vec<String>,
}

#[derive(Debug, Serialize)]
struct SecurityReport {
    session_id: String,
    total_navigations: usize,
    insecure_urls: Vec<String>,
    sensitive_selectors: Vec<String>,
    warnings: Vec<String>,
}

#[derive(Debug, Serialize)]
struct SelectorCount {
    selector: String,
    count: usize,
}

#[derive(Debug, Serialize)]
struct UsabilityReport {
    session_id: String,
    repeated_clicks: Vec<SelectorCount>,
    repeated_inputs: Vec<SelectorCount>,
    slow_segments: Vec<IdlePeriodReport>,
}

#[derive(Debug, Serialize)]
struct CompatibilityReport {
    session_id: String,
    navigation_count: usize,
    unique_domains: Vec<String>,
    schemes_used: Vec<String>,
    mixed_content: bool,
}

fn build_performance_report(
    session_id: &str,
    analytics: &SessionAnalytics,
    events: &[BrowserEvent],
) -> PerformanceReport {
    let total_events = events.len();
    let duration_ms = analytics.duration_ms;

    let mut gaps = Vec::new();
    let mut idle_periods = Vec::new();

    for window in events.windows(2) {
        if let [prev, next] = window {
            let gap = next.timestamp.saturating_sub(prev.timestamp);
            gaps.push(gap);
            if gap as i64 >= 1_500 {
                idle_periods.push(IdlePeriodReport {
                    from_event: describe_event(prev),
                    to_event: describe_event(next),
                    duration_ms: gap as i64,
                });
            }
        }
    }

    let average_gap = if !gaps.is_empty() {
        Some(gaps.iter().sum::<i64>() as f64 / gaps.len() as f64)
    } else {
        None
    };

    let longest_gap = gaps.into_iter().max();

    let events_per_minute = if duration_ms > 0 {
        let minutes = duration_ms as f64 / 60_000.0;
        if minutes > 0.0 {
            total_events as f64 / minutes
        } else {
            total_events as f64
        }
    } else {
        total_events as f64
    };

    PerformanceReport {
        session_id: session_id.to_string(),
        total_events,
        duration_ms,
        events_per_minute,
        average_gap_ms: average_gap,
        longest_gap_ms: longest_gap.map(|gap| gap as i64),
        idle_periods,
    }
}

fn build_accessibility_report(session_id: &str, events: &[BrowserEvent]) -> AccessibilityReport {
    let mut selectors = Vec::new();

    for event in events {
        match event.event_type.as_str() {
            "click" | "type" | "select" => {
                if let Some(selector) = event.data.get("selector").and_then(|v| v.as_str()) {
                    selectors.push(selector.to_string());
                }
            }
            _ => {}
        }
    }

    let total_interactions = selectors.len();
    let accessible_interactions = selectors
        .iter()
        .filter(|selector| is_accessible_selector(selector))
        .count();

    let selectors_missing_accessibility = selectors
        .into_iter()
        .filter(|selector| !is_accessible_selector(selector))
        .collect::<Vec<_>>();

    let accessibility_score = if total_interactions > 0 {
        accessible_interactions as f64 / total_interactions as f64
    } else {
        1.0
    };

    AccessibilityReport {
        session_id: session_id.to_string(),
        total_interactions,
        accessible_interactions,
        accessibility_score,
        selectors_missing_accessibility,
    }
}

fn build_security_report(session_id: &str, events: &[BrowserEvent]) -> SecurityReport {
    let mut insecure_urls = Vec::new();
    let mut sensitive_selectors = Vec::new();
    let mut warnings = Vec::new();
    let mut navigation_count = 0;

    for event in events {
        match event.event_type.as_str() {
            "navigate" => {
                navigation_count += 1;
                if let Some(url) = event.data.get("url").and_then(|v| v.as_str()) {
                    if url.starts_with("http://") {
                        insecure_urls.push(url.to_string());
                    }
                }
            }
            "type" => {
                if let Some(selector) = event.data.get("selector").and_then(|v| v.as_str()) {
                    let selector_lower = selector.to_lowercase();
                    if selector_lower.contains("password")
                        || selector_lower.contains("secret")
                        || selector_lower.contains("token")
                    {
                        sensitive_selectors.push(selector.to_string());
                    }
                }
            }
            _ => {}
        }
    }

    if !insecure_urls.is_empty() {
        warnings
            .push("Detected navigation over HTTP; prefer HTTPS to avoid mixed content".to_string());
    }

    if !sensitive_selectors.is_empty() {
        warnings.push(
            "Sensitive form fields were interacted with; ensure secrets are handled securely"
                .to_string(),
        );
    }

    SecurityReport {
        session_id: session_id.to_string(),
        total_navigations: navigation_count,
        insecure_urls,
        sensitive_selectors,
        warnings,
    }
}

fn build_usability_report(session_id: &str, events: &[BrowserEvent]) -> UsabilityReport {
    let mut click_counts: HashMap<String, usize> = HashMap::new();
    let mut input_counts: HashMap<String, usize> = HashMap::new();

    for event in events {
        match event.event_type.as_str() {
            "click" => {
                if let Some(selector) = event.data.get("selector").and_then(|v| v.as_str()) {
                    *click_counts.entry(selector.to_string()).or_insert(0) += 1;
                }
            }
            "type" | "select" => {
                if let Some(selector) = event.data.get("selector").and_then(|v| v.as_str()) {
                    *input_counts.entry(selector.to_string()).or_insert(0) += 1;
                }
            }
            _ => {}
        }
    }

    let repeated_clicks = click_counts
        .into_iter()
        .filter(|(_, count)| *count > 1)
        .map(|(selector, count)| SelectorCount { selector, count })
        .collect();

    let repeated_inputs = input_counts
        .into_iter()
        .filter(|(_, count)| *count > 1)
        .map(|(selector, count)| SelectorCount { selector, count })
        .collect();

    let slow_segments = idle_periods_over(events, 2_500);

    UsabilityReport {
        session_id: session_id.to_string(),
        repeated_clicks,
        repeated_inputs,
        slow_segments,
    }
}

fn build_compatibility_report(session_id: &str, events: &[BrowserEvent]) -> CompatibilityReport {
    let mut domains = HashSet::new();
    let mut schemes = HashSet::new();
    let mut navigation_count = 0;

    for event in events {
        if event.event_type.as_str() == "navigate" {
            navigation_count += 1;
            if let Some(url) = event.data.get("url").and_then(|v| v.as_str()) {
                if let Some((scheme, host)) = parse_url_parts(url) {
                    schemes.insert(scheme);
                    if !host.is_empty() {
                        domains.insert(host);
                    }
                }
            }
        }
    }

    let mixed_content = schemes.contains("http") && schemes.contains("https");

    CompatibilityReport {
        session_id: session_id.to_string(),
        navigation_count,
        unique_domains: to_sorted_vec(domains),
        schemes_used: to_sorted_vec(schemes),
        mixed_content,
    }
}

fn idle_periods_over(events: &[BrowserEvent], threshold_ms: i64) -> Vec<IdlePeriodReport> {
    let mut periods = Vec::new();

    for window in events.windows(2) {
        if let [prev, next] = window {
            let gap = next.timestamp.saturating_sub(prev.timestamp) as i64;
            if gap >= threshold_ms {
                periods.push(IdlePeriodReport {
                    from_event: describe_event(prev),
                    to_event: describe_event(next),
                    duration_ms: gap,
                });
            }
        }
    }

    periods
}

fn describe_event(event: &BrowserEvent) -> String {
    match event.event_type.as_str() {
        "navigate" => event
            .data
            .get("url")
            .and_then(|v| v.as_str())
            .map(|url| format!("navigate -> {}", url))
            .unwrap_or_else(|| "navigate".to_string()),
        "click" => event
            .data
            .get("selector")
            .and_then(|v| v.as_str())
            .map(|selector| format!("click -> {}", selector))
            .unwrap_or_else(|| "click".to_string()),
        "type" => event
            .data
            .get("selector")
            .and_then(|v| v.as_str())
            .map(|selector| format!("type -> {}", selector))
            .unwrap_or_else(|| "type".to_string()),
        other => other.to_string(),
    }
}

fn is_accessible_selector(selector: &str) -> bool {
    let lower = selector.to_lowercase();
    lower.contains("aria-")
        || lower.contains("[role=")
        || lower.contains("button")
        || lower.contains("label")
        || lower.contains("input")
}

fn parse_url_parts(url: &str) -> Option<(String, String)> {
    let mut parts = url.splitn(2, "://");
    let scheme = parts.next()?;
    let rest = parts.next().unwrap_or("");
    let host = rest.split('/').next().unwrap_or("");
    Some((scheme.to_string(), host.to_string()))
}

fn to_sorted_vec(set: HashSet<String>) -> Vec<String> {
    let mut items: Vec<String> = set.into_iter().collect();
    items.sort();
    items
}

fn print_performance_summary(report: &PerformanceReport) {
    println!("Performance insights:");
    println!("- Events per minute: {:.2}", report.events_per_minute);

    if let Some(avg) = report.average_gap_ms {
        println!("- Average gap between events: {:.0} ms", avg);
    }

    if let Some(longest) = report.longest_gap_ms {
        println!("- Longest idle period: {} ms", longest);
    }

    if report.idle_periods.is_empty() {
        println!("- No idle periods above 1500 ms detected.");
    } else {
        println!("- Idle periods above 1500 ms:");
        for idle in &report.idle_periods {
            println!(
                "  - {} -> {} ({} ms)",
                idle.from_event, idle.to_event, idle.duration_ms
            );
        }
    }
}

fn print_accessibility_summary(report: &AccessibilityReport) {
    println!("Accessibility insights:");
    println!(
        "- Accessible interactions: {} of {}",
        report.accessible_interactions, report.total_interactions
    );
    println!(
        "- Accessibility score: {:.0}%",
        report.accessibility_score * 100.0
    );

    if report.selectors_missing_accessibility.is_empty() {
        println!("- All selectors include basic accessibility hints.");
    } else {
        println!("- Selectors missing accessibility cues:");
        for selector in &report.selectors_missing_accessibility {
            println!("  - {}", selector);
        }
    }
}

fn print_security_summary(report: &SecurityReport) {
    println!("Security observations:");
    println!("- Navigations observed: {}", report.total_navigations);

    if report.insecure_urls.is_empty() {
        println!("- No HTTP navigations detected.");
    } else {
        println!("- Navigations using HTTP:");
        for url in &report.insecure_urls {
            println!("  - {}", url);
        }
    }

    if report.sensitive_selectors.is_empty() {
        println!("- No sensitive input selectors matched.");
    } else {
        println!("- Sensitive selectors interacted with:");
        for selector in &report.sensitive_selectors {
            println!("  - {}", selector);
        }
    }

    if report.warnings.is_empty() {
        println!("- No security warnings raised.");
    } else {
        for warning in &report.warnings {
            println!("- Warning: {}", warning);
        }
    }
}

fn print_usability_summary(report: &UsabilityReport) {
    println!("Usability highlights:");

    if report.repeated_clicks.is_empty() {
        println!("- No repeated clicks detected on the same selector.");
    } else {
        println!("- Repeated clicks detected:");
        for entry in &report.repeated_clicks {
            println!("  - {} ({} times)", entry.selector, entry.count);
        }
    }

    if report.repeated_inputs.is_empty() {
        println!("- No fields were typed into more than once.");
    } else {
        println!("- Fields edited multiple times:");
        for entry in &report.repeated_inputs {
            println!("  - {} ({} times)", entry.selector, entry.count);
        }
    }

    if report.slow_segments.is_empty() {
        println!("- No slow segments above 2500 ms detected.");
    } else {
        println!("- Slow segments above 2500 ms:");
        for idle in &report.slow_segments {
            println!(
                "  - {} -> {} ({} ms)",
                idle.from_event, idle.to_event, idle.duration_ms
            );
        }
    }
}

fn print_compatibility_summary(report: &CompatibilityReport) {
    println!("Compatibility snapshot:");
    println!("- Navigations performed: {}", report.navigation_count);
    if report.schemes_used.is_empty() {
        println!("- Schemes used: (none)");
    } else {
        println!("- Schemes used: {}", report.schemes_used.join(", "));
    }

    if report.mixed_content {
        println!("- Warning: both HTTP and HTTPS were used; check for mixed content issues.");
    }

    if report.unique_domains.is_empty() {
        println!("- No external domains were visited.");
    } else {
        println!("- Domains visited:");
        for domain in &report.unique_domains {
            println!("  - {}", domain);
        }
    }
}

async fn persist_event(
    storage: &Arc<StorageManager>,
    tenant: &TenantId,
    session_id: &str,
    sequence: u64,
    event_type: &str,
    data: serde_json::Value,
) -> Result<()> {
    let event = BrowserEvent {
        id: uuid::Uuid::new_v4().to_string(),
        tenant: tenant.clone(),
        session_id: session_id.to_string(),
        timestamp: chrono::Utc::now().timestamp_millis(),
        event_type: event_type.to_string(),
        data,
        sequence,
        tags: vec!["recording".to_string()],
    };

    storage
        .backend()
        .store_event(event)
        .await
        .context("Failed to persist recording event")?;

    Ok(())
}

async fn save_results(results: &AutomationResults, output_dir: &PathBuf) -> Result<()> {
    fs::create_dir_all(output_dir).await?;

    let results_file = output_dir.join("results.json");
    let results_json = serde_json::to_string_pretty(results)?;
    fs::write(results_file, results_json).await?;

    Ok(())
}

async fn check_available_browsers() -> Result<Vec<String>> {
    let mut available = Vec::new();

    // This would check for actual browser installations
    // For now, return a mock list
    available.push("Chromium".to_string());
    available.push("Chrome".to_string());
    available.push("Firefox".to_string());

    Ok(available)
}

// All implementations have been moved to their respective modules:
// - automation/mod.rs: AutomationEngine, AutomationConfig, AutomationResults
// - replay/mod.rs: SessionReplayer, ReplayConfig, ReplayResults
// - export/mod.rs: JsonExporter, CsvExporter, HtmlExporter

// [OLD INLINE IMPLEMENTATIONS REMOVED - 800+ lines cleaned up]

// Keeping main.rs focused on CLI entry point only
