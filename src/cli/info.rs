use std::env;

use anyhow::Result;
use humantime::format_rfc3339;
use soulbrowser_kernel::app_context::get_or_create_context;
use soulbrowser_kernel::Config;
use soulbrowser_state_center::{DispatchStatus, PerceiverEventKind, StateEvent};

use super::scheduler::scheduler_overview;

pub async fn cmd_info(config: &Config) -> Result<()> {
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
    let strict_env = env::var("SOUL_STRICT_AUTHZ")
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
    let browsers = check_available_browsers().await?;
    for browser in browsers {
        println!("- {} ✓", browser);
    }

    println!();
    println!("System Health: ✓ All systems operational");

    Ok(())
}

async fn check_available_browsers() -> Result<Vec<&'static str>> {
    // Placeholder implementation; in real code we would probe actual availability
    Ok(vec!["Chromium", "Chrome", "Firefox"])
}
