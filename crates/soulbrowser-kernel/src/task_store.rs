use crate::agent::ChatSessionOutput;
use crate::task_status::TaskAnnotation;
use crate::visualization::build_plan_overlays;
use anyhow::{Context, Result};
use chrono::{DateTime, Duration as ChronoDuration, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use tokio::fs;
use tokio::io::AsyncReadExt;
use tracing::warn;

/// Indicates where a persisted plan originated.
#[derive(Clone, Copy, Debug)]
pub enum PlanSource {
    ApiChat,
    CliChat,
    TaskCenter,
}

impl PlanSource {
    fn as_str(&self) -> &'static str {
        match self {
            PlanSource::ApiChat => "api_chat",
            PlanSource::CliChat => "cli_chat",
            PlanSource::TaskCenter => "task_center",
        }
    }
}

/// JSON payload saved for each generated plan.
fn default_planner() -> String {
    "rule".to_string()
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PersistedPlanRecord {
    pub version: u32,
    pub task_id: String,
    pub prompt: String,
    pub created_at: String,
    pub source: String,
    pub plan: Value,
    pub flow: Value,
    pub explanations: Vec<String>,
    pub summary: Vec<String>,
    pub constraints: Vec<String>,
    pub current_url: Option<String>,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default = "default_planner")]
    pub planner: String,
    pub llm_provider: Option<String>,
    pub llm_model: Option<String>,
    #[serde(default)]
    pub context_snapshot: Option<Value>,
    #[serde(default)]
    pub annotations: Option<Vec<TaskAnnotation>>,
}

#[derive(Debug, Clone)]
pub struct PlanOriginMetadata {
    pub planner: String,
    pub llm_provider: Option<String>,
    pub llm_model: Option<String>,
}

impl PersistedPlanRecord {
    pub fn from_session(
        session: &ChatSessionOutput,
        prompt: &str,
        constraints: Vec<String>,
        current_url: Option<String>,
        source: PlanSource,
        origin: PlanOriginMetadata,
        context_snapshot: Option<Value>,
        session_id: Option<&str>,
    ) -> Result<Self> {
        let mut plan_json =
            serde_json::to_value(&session.plan).with_context(|| "serializing agent plan")?;
        if let Value::Object(ref mut obj) = plan_json {
            obj.insert("overlays".to_string(), build_plan_overlays(&session.plan));
        }
        let flow_definition = serde_json::to_value(&session.flow.flow)
            .with_context(|| "serializing flow definition")?;
        let flow_json = json!({
            "definition": flow_definition,
            "metadata": {
                "step_count": session.flow.step_count,
                "validation_count": session.flow.validation_count,
            }
        });

        Ok(Self {
            version: 1,
            task_id: session.plan.task_id.0.clone(),
            prompt: prompt.to_string(),
            created_at: session
                .plan
                .created_at
                .to_rfc3339_opts(SecondsFormat::Secs, true),
            source: source.as_str().to_string(),
            plan: plan_json,
            flow: flow_json,
            explanations: session.explanations.clone(),
            summary: session.summarize_steps(),
            constraints,
            current_url,
            session_id: session_id.map(|s| s.to_string()),
            planner: origin.planner,
            llm_provider: origin.llm_provider,
            llm_model: origin.llm_model,
            context_snapshot,
            annotations: None,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanSummary {
    pub task_id: String,
    pub prompt: String,
    pub created_at: String,
    pub source: String,
    pub path: PathBuf,
    pub planner: Option<String>,
    pub llm_provider: Option<String>,
    pub llm_model: Option<String>,
    pub session_id: Option<String>,
}

/// Simple filesystem-backed store for task plans.
pub struct TaskPlanStore {
    root: PathBuf,
}

impl TaskPlanStore {
    pub fn new<P: Into<PathBuf>>(base_dir: P) -> Self {
        Self {
            root: base_dir.into(),
        }
    }

    fn tasks_dir(&self) -> PathBuf {
        self.root.join("tasks")
    }

    fn task_path(&self, task_id: &str) -> PathBuf {
        self.tasks_dir().join(format!("{}.json", task_id))
    }

    pub async fn prune_expired(&self, max_age: ChronoDuration) -> Result<usize> {
        if max_age <= ChronoDuration::zero() {
            return Ok(0);
        }
        let dir = self.tasks_dir();
        if fs::metadata(&dir).await.is_err() {
            return Ok(0);
        }
        let cutoff = Utc::now() - max_age;
        let mut removed = 0usize;
        let mut entries = fs::read_dir(&dir)
            .await
            .with_context(|| format!("reading task directory {}", dir.display()))?;
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            match Self::peek_created_at(&path).await {
                Ok(Some(created_at)) if created_at < cutoff => match fs::remove_file(&path).await {
                    Ok(_) => removed += 1,
                    Err(err) => warn!(
                        ?err,
                        path = %path.display(),
                        "failed to remove expired plan file"
                    ),
                },
                Ok(_) => {}
                Err(err) => warn!(?err, path = %path.display(), "failed to inspect plan file"),
            }
        }
        Ok(removed)
    }

    pub async fn save_plan(&self, record: &PersistedPlanRecord) -> Result<PathBuf> {
        let dir = self.tasks_dir();
        fs::create_dir_all(&dir)
            .await
            .with_context(|| format!("creating task plan directory {}", dir.display()))?;
        let path = self.task_path(&record.task_id);
        let payload = serde_json::to_vec_pretty(record)?;
        fs::write(&path, payload)
            .await
            .with_context(|| format!("writing task plan to {}", path.display()))?;
        Ok(path)
    }

    pub async fn list_plan_summaries(&self) -> Result<Vec<PlanSummary>> {
        let dir = self.tasks_dir();
        if fs::metadata(&dir).await.is_err() {
            return Ok(Vec::new());
        }

        let mut entries = fs::read_dir(&dir)
            .await
            .with_context(|| format!("reading task directory {}", dir.display()))?;
        let mut summaries = Vec::new();
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            match Self::load_plan_path(&path).await {
                Ok(record) => summaries.push(PlanSummary {
                    task_id: record.task_id.clone(),
                    prompt: record.prompt.clone(),
                    created_at: record.created_at.clone(),
                    source: record.source.clone(),
                    path,
                    planner: Some(record.planner.clone()),
                    llm_provider: record.llm_provider.clone(),
                    llm_model: record.llm_model.clone(),
                    session_id: record.session_id.clone(),
                }),
                Err(err) => {
                    tracing::warn!(?err, path = %path.display(), "failed to read task plan file");
                }
            }
        }
        Ok(summaries)
    }

    pub async fn load_plan(&self, task_id: &str) -> Result<PersistedPlanRecord> {
        let path = self.task_path(task_id);
        Self::load_plan_path(&path).await
    }

    async fn load_plan_path(path: &PathBuf) -> Result<PersistedPlanRecord> {
        let mut file = fs::File::open(path)
            .await
            .with_context(|| format!("opening plan {}", path.display()))?;
        let mut buf = Vec::new();
        file.read_to_end(&mut buf).await?;
        let record = serde_json::from_slice(&buf)
            .with_context(|| format!("parsing plan {}", path.display()))?;
        Ok(record)
    }

    async fn peek_created_at(path: &PathBuf) -> Result<Option<DateTime<Utc>>> {
        let record = match Self::load_plan_path(path).await {
            Ok(record) => record,
            Err(err) => {
                warn!(?err, path = %path.display(), "failed to parse plan file for ttl");
                return Ok(None);
            }
        };
        match DateTime::parse_from_rfc3339(&record.created_at) {
            Ok(dt) => Ok(Some(dt.with_timezone(&Utc))),
            Err(err) => {
                warn!(
                    ?err,
                    path = %path.display(),
                    created_at = %record.created_at,
                    "failed to parse plan created_at"
                );
                Ok(None)
            }
        }
    }
}

/// Remove execution output bundles under `soulbrowser-output/tasks/<task_id>` that
/// are older than the configured TTL.
pub async fn prune_execution_outputs(base_dir: &Path, max_age: ChronoDuration) -> Result<usize> {
    if max_age <= ChronoDuration::zero() {
        return Ok(0);
    }
    let dir = base_dir.join("tasks");
    if fs::metadata(&dir).await.is_err() {
        return Ok(0);
    }

    let cutoff = Utc::now() - max_age;
    let mut removed = 0usize;
    let mut entries = fs::read_dir(&dir)
        .await
        .with_context(|| format!("reading execution output directory {}", dir.display()))?;

    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        let metadata = match entry.metadata().await {
            Ok(value) => value,
            Err(err) => {
                warn!(?err, path = %path.display(), "failed to read metadata for output directory");
                continue;
            }
        };
        if !metadata.is_dir() {
            continue;
        }
        let modified = match metadata.modified().ok().map(DateTime::<Utc>::from) {
            Some(value) => value,
            None => continue,
        };
        if modified >= cutoff {
            continue;
        }
        match fs::remove_dir_all(&path).await {
            Ok(_) => removed += 1,
            Err(err) => warn!(
                ?err,
                path = %path.display(),
                "failed to remove expired execution output directory"
            ),
        }
    }

    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;
    use tokio::time::{sleep, Duration};

    #[tokio::test]
    async fn prunes_old_execution_outputs() {
        let tmp = tempdir().expect("temp dir");
        let base = tmp.path();
        let tasks_dir = base.join("tasks");

        fs::create_dir_all(tasks_dir.join("old"))
            .await
            .expect("create old dir");
        // Ensure the second directory has a newer modified timestamp
        sleep(Duration::from_millis(1100)).await;
        fs::create_dir_all(tasks_dir.join("recent"))
            .await
            .expect("create recent dir");

        let removed = prune_execution_outputs(base, ChronoDuration::seconds(1))
            .await
            .expect("prune outputs");
        assert_eq!(removed, 1);
        assert!(!tasks_dir.join("old").exists());
        assert!(tasks_dir.join("recent").exists());
    }
}
