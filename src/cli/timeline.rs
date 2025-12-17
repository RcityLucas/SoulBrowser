use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use crate::cli::context::CliContext;
use anyhow::{anyhow, bail, Context, Result};
use chrono::{offset::LocalResult, TimeZone, Utc};
use clap::{Args, ValueEnum};
use l6_timeline::{
    api::TimelineService as L6TimelineService,
    model::{By as TimelineBy, ExportReq as TimelineExportReq, View as TimelineView},
    policy::{set_policy as timeline_set_policy, TimelinePolicyView},
    Timeline,
};
use soulbrowser_core_types::{ActionId, SessionId};
use soulbrowser_event_store::api::InMemoryEventStore as TimelineEventStore;
use soulbrowser_event_store::model::{
    AppendMeta as TimelineAppendMeta, EventEnvelope as TimelineEventEnvelope,
    EventScope as TimelineEventScope, EventSource as TimelineEventSource,
    LogLevel as TimelineLogLevel,
};
use soulbrowser_event_store::{
    EsPolicyView as TimelineEsPolicy, EventStore as TimelineEventStoreTrait,
};
use soulbrowser_kernel::storage::{BrowserEvent, QueryParams, StorageManager};

#[derive(Clone, Debug, ValueEnum)]
pub enum TimelineViewOpt {
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

#[derive(Args, Clone)]
pub struct TimelineArgs {
    #[arg(long, value_enum, default_value = "records")]
    pub view: TimelineViewOpt,

    #[arg(long)]
    pub action_id: Option<String>,

    #[arg(long)]
    pub flow_id: Option<String>,

    #[arg(long)]
    pub task_id: Option<String>,

    #[arg(long)]
    pub since: Option<String>,

    #[arg(long)]
    pub until: Option<String>,

    #[arg(long, default_value_t = 5000)]
    pub limit: usize,

    #[arg(long)]
    pub max_lines: Option<usize>,

    #[arg(long)]
    pub output: Option<PathBuf>,
}

pub async fn cmd_timeline(args: TimelineArgs, ctx: &CliContext) -> Result<()> {
    let context = ctx
        .app_context_with("cli-timeline", Some(ctx.config().output_dir.clone()))
        .await?;

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
            fs::create_dir_all(parent).with_context(|| {
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
            let payload = lines.join("\n");
            fs::write(path, format!("{}\n", payload))
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
        bail!(
            "Specify exactly one selector: --action-id, --flow-id, --task-id or ( --since + --until )"
        );
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
