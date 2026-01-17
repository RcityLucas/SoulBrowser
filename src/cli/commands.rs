use clap::Subcommand;

use super::analyze::AnalyzeArgs;
use super::artifacts::ArtifactsArgs;
use super::chat::ChatArgs;
use super::config::ConfigArgs;
use super::console::ConsoleArgs;
use super::demo::DemoArgs;
use super::export::ExportArgs;
use super::gateway::GatewayArgs;
use super::perceive::PerceiveArgs;
use super::perceiver::PerceiverArgs;
use super::policy::PolicyArgs;
use super::record::RecordArgs;
use super::replay::ReplayArgs;
use super::run::RunArgs;
use super::scheduler::SchedulerArgs;
use super::serve::ServeArgs;
use super::start::StartArgs;
use super::telemetry::TelemetryArgs;
use super::timeline::TimelineArgs;
use super::tools::ToolsArgs;

#[derive(Subcommand, Clone)]
pub enum Commands {
    /// Start an interactive browser session
    Start(StartArgs),

    /// Run automation scripts
    Run(RunArgs),

    /// Record user interactions for later replay
    Record(RecordArgs),

    /// Replay a recorded session or run bundle
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

    /// Manage tool registry entries (list/register/remove)
    Tools(ToolsArgs),

    /// Telemetry utilities (e.g. tail live events)
    Telemetry(TelemetryArgs),
}
