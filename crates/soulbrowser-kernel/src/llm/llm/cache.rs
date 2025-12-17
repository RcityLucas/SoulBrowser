use agent_core::AgentPlan;
use anyhow::anyhow;
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::fs;
use tokio::io::AsyncWriteExt;
use tracing::{debug, warn};

use crate::metrics;

#[derive(Clone)]
pub struct LlmPlanCache {
    root: PathBuf,
    namespace_label: String,
}

impl LlmPlanCache {
    pub fn new(root: PathBuf) -> anyhow::Result<Self> {
        Self::with_namespace(root, "default")
    }

    pub fn with_namespace(
        root: PathBuf,
        namespace_label: impl Into<String>,
    ) -> anyhow::Result<Self> {
        std::fs::create_dir_all(&root)?;
        Ok(Self {
            root,
            namespace_label: namespace_label.into(),
        })
    }

    pub async fn load_plan(&self, key: &str) -> Option<CachedPlan> {
        let path = self.entry_path(key);
        match fs::read(&path).await {
            Ok(bytes) => match serde_json::from_slice::<CachedPlan>(&bytes) {
                Ok(plan) => {
                    debug!(path = %path.display(), "LLM cache hit");
                    metrics::record_llm_cache_event(&self.namespace_label, "hit");
                    Some(plan)
                }
                Err(err) => {
                    warn!(%err, path = %path.display(), "failed to parse cached plan");
                    metrics::record_llm_cache_event(&self.namespace_label, "error");
                    None
                }
            },
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                metrics::record_llm_cache_event(&self.namespace_label, "miss");
                None
            }
            Err(err) => {
                warn!(%err, path = %path.display(), "failed to read cached plan");
                metrics::record_llm_cache_event(&self.namespace_label, "error");
                None
            }
        }
    }

    pub async fn store_plan(&self, key: &str, plan: &AgentPlan, explanations: &[String]) {
        let path = self.entry_path(key);
        if let Some(parent) = path.parent() {
            if let Err(err) = fs::create_dir_all(parent).await {
                warn!(%err, path = %parent.display(), "failed to create cache directory");
                return;
            }
        }
        let record = CachedPlan {
            plan: plan.clone(),
            explanations: explanations.to_vec(),
        };
        match serde_json::to_vec_pretty(&record) {
            Ok(payload) => {
                if let Err(err) = write_atomic(&path, &payload).await {
                    warn!(%err, path = %path.display(), "failed to write cached plan");
                    metrics::record_llm_cache_event(&self.namespace_label, "error");
                } else {
                    debug!(path = %path.display(), "LLM cache store");
                    metrics::record_llm_cache_event(&self.namespace_label, "store");
                }
            }
            Err(err) => warn!(%err, path = %path.display(), "failed to serialize cached plan"),
        }
    }

    fn entry_path(&self, key: &str) -> PathBuf {
        let shard = &key[..2.min(key.len())];
        self.root.join(shard).join(format!("{key}.json"))
    }
}

async fn write_atomic(path: &Path, data: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension("tmp");
    let mut file = fs::File::create(&tmp).await?;
    file.write_all(data).await?;
    file.flush().await?;
    fs::rename(tmp, path).await
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedPlan {
    pub plan: AgentPlan,
    pub explanations: Vec<String>,
}

#[derive(Clone)]
pub struct LlmCachePool {
    root: PathBuf,
    caches: Arc<DashMap<String, Arc<LlmPlanCache>>>,
}

impl LlmCachePool {
    pub fn new(root: PathBuf) -> anyhow::Result<Self> {
        std::fs::create_dir_all(&root)?;
        Ok(Self {
            root,
            caches: Arc::new(DashMap::new()),
        })
    }

    pub fn scoped(&self, components: &[&str]) -> anyhow::Result<Arc<LlmPlanCache>> {
        if components.is_empty() {
            return Err(anyhow!(
                "LLM cache namespace requires at least one component"
            ));
        }

        let normalized: Vec<String> = components
            .iter()
            .map(|value| sanitize_component(value))
            .collect();
        let key = normalized.join("/");
        if let Some(cache) = self.caches.get(&key) {
            return Ok(cache.clone());
        }

        let mut path = self.root.clone();
        for component in &normalized {
            path.push(component);
        }
        let label = normalized.join("::");
        let cache = Arc::new(LlmPlanCache::with_namespace(path, label)?);
        Ok(self
            .caches
            .entry(key)
            .or_insert_with(|| cache.clone())
            .clone())
    }
}

fn sanitize_component(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return "default".to_string();
    }
    trimmed
        .chars()
        .map(|ch| match ch {
            'a'..='z' | 'A'..='Z' | '0'..='9' => ch,
            '-' | '_' => ch,
            _ => '-',
        })
        .collect()
}
