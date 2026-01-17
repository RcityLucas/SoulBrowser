use super::analyze::cmd_analyze;
use super::artifacts::cmd_artifacts;
use super::chat::cmd_chat;
use super::config::cmd_config;
use super::console::cmd_console;
use super::demo::cmd_demo_real;
use super::env::CliArgs;
use super::export::cmd_export;
use super::gateway::cmd_gateway;
use super::info::cmd_info;
use super::perceive::cmd_perceive;
use super::perceiver::cmd_perceiver;
use super::policy::cmd_policy;
use super::record::cmd_record;
use super::replay::cmd_replay;
use super::run::cmd_run;
use super::scheduler::cmd_scheduler;
use super::serve::cmd_serve;
use super::start::cmd_start;
use super::telemetry::cmd_telemetry;
use super::timeline::cmd_timeline;
use super::tools::cmd_tools;
use crate::cli::commands::Commands;
use crate::cli::context::CliContext;
use anyhow::Result;

pub async fn dispatch(cli: &CliArgs, ctx: &CliContext) -> Result<()> {
    match cli.command.clone() {
        Commands::Start(args) => cmd_start(args, ctx).await,
        Commands::Run(args) => cmd_run(args, ctx.config()).await,
        Commands::Record(args) => cmd_record(args, ctx).await,
        Commands::Replay(args) => cmd_replay(args, ctx.config()).await,
        Commands::Export(args) => cmd_export(args, ctx.config()).await,
        Commands::Analyze(args) => cmd_analyze(args, ctx).await,
        Commands::Chat(args) => cmd_chat(args, ctx, cli.output.clone()).await,
        Commands::Config(args) => cmd_config(args, ctx).await,
        Commands::Info => cmd_info(ctx.config()).await,
        Commands::Perceiver(args) => cmd_perceiver(args, ctx).await,
        Commands::Scheduler(args) => cmd_scheduler(args, ctx.config()).await,
        Commands::Policy(args) => cmd_policy(args, ctx.config()).await,
        Commands::Artifacts(args) => cmd_artifacts(args).await,
        Commands::Console(args) => cmd_console(args).await,
        Commands::Timeline(args) => cmd_timeline(args, ctx).await,
        Commands::Gateway(args) => cmd_gateway(args, ctx.config()).await,
        Commands::Demo(args) => cmd_demo_real(args).await,
        Commands::Perceive(args) => cmd_perceive(args, ctx).await,
        Commands::Serve(args) => cmd_serve(args, ctx.metrics_port(), ctx.config().clone()).await,
        Commands::Tools(args) => cmd_tools(args, ctx).await,
        Commands::Telemetry(args) => cmd_telemetry(args, ctx).await,
    }
}
