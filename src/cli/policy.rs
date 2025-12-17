use anyhow::{anyhow, Result};
use clap::{Args, Subcommand};
use serde_json::json;
use soulbrowser_kernel::app_context::get_or_create_context;
use soulbrowser_kernel::Config;
use soulbrowser_policy_center::RuntimeOverrideSpec;

use super::scheduler::{scheduler_json_payload, scheduler_overview};

#[derive(Args, Clone, Debug)]
pub struct PolicyArgs {
    #[command(subcommand)]
    pub command: PolicyCommand,
}

#[derive(Subcommand, Clone, Debug)]
pub enum PolicyCommand {
    Show(PolicyShowArgs),
    Override(PolicyOverrideArgs),
}

#[derive(Args, Clone, Debug)]
pub struct PolicyShowArgs {
    /// Output JSON instead of human summary
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Clone, Debug)]
pub struct PolicyOverrideArgs {
    /// Dot-path to override, e.g. scheduler.limits.global_slots
    pub path: String,
    /// Override value as JSON literal (e.g. 4, true, "value")
    pub value: String,
    /// Override owner label
    #[arg(long, default_value = "cli")]
    pub owner: String,
    /// Reason for override
    #[arg(long, default_value = "manual override")]
    pub reason: String,
    /// TTL in seconds (0 = permanent)
    #[arg(long)]
    pub ttl: Option<u64>,
}

pub async fn cmd_policy(args: PolicyArgs, config: &Config) -> Result<()> {
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
                let payload = json!({
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
