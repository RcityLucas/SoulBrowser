use chrono::Utc;
use parking_lot::RwLock;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use tracing::warn;

#[derive(Debug)]
pub struct PluginRegistry {
    entries: RwLock<HashMap<String, PluginRecord>>,
    source_path: Option<PathBuf>,
}

#[derive(Debug)]
pub enum RegistryError {
    PluginNotFound(String),
    HelperExists(String),
    HelperNotFound(String),
    HelperMissingSteps(String),
}

impl fmt::Display for RegistryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RegistryError::PluginNotFound(id) => write!(f, "plugin '{}' not found", id),
            RegistryError::HelperExists(id) => write!(f, "helper '{}' already exists", id),
            RegistryError::HelperNotFound(id) => write!(f, "helper '{}' not found", id),
            RegistryError::HelperMissingSteps(id) => {
                write!(f, "helper '{}' must define at least one step", id)
            }
        }
    }
}

impl std::error::Error for RegistryError {}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PluginRecord {
    pub id: String,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub owner: Option<String>,
    #[serde(default)]
    pub last_reviewed_at: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub scopes: Option<Vec<String>>,
    #[serde(default)]
    pub helpers: Vec<RegistryHelper>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct PluginRegistryStats {
    pub total_registry_entries: usize,
    pub active_plugins: usize,
    pub pending_review: usize,
    pub last_reviewed_at: Option<String>,
    pub registry_path: Option<String>,
}

impl PluginRegistry {
    pub fn load_default() -> Self {
        let primary = PathBuf::from("config/plugins/registry.json");
        if primary.exists() {
            return Self::load_from_path(primary);
        }
        let example = PathBuf::from("config/plugins/registry.example.json");
        if example.exists() {
            if let Ok(bytes) = fs::read(&example) {
                if let Ok(file) = serde_json::from_slice::<PluginRegistryFile>(&bytes) {
                    return Self::from_records(file.plugins, Some(primary));
                }
            }
        }
        Self::from_records(Vec::new(), Some(primary))
    }

    pub fn load_from_path(path: PathBuf) -> Self {
        if path.exists() {
            if let Ok(bytes) = fs::read(&path) {
                if let Ok(file) = serde_json::from_slice::<PluginRegistryFile>(&bytes) {
                    return Self::from_records(file.plugins, Some(path));
                }
            }
        }
        Self::from_records(Vec::new(), Some(path))
    }

    fn from_records(records: Vec<PluginRecord>, path: Option<PathBuf>) -> Self {
        let mut map: HashMap<String, PluginRecord> = records
            .into_iter()
            .map(|record| (record.id.clone(), record))
            .collect();
        for (id, record) in map.iter_mut() {
            for helper in &mut record.helpers {
                helper.normalize(id);
            }
        }
        Self {
            entries: RwLock::new(map),
            source_path: path,
        }
    }

    pub fn is_active(&self, plugin_id: &str) -> bool {
        self.entries
            .read()
            .get(plugin_id)
            .map(|record| matches!(record.status.as_deref(), Some(status) if status.eq_ignore_ascii_case("active")))
            .unwrap_or(true)
    }

    pub fn stats(&self) -> PluginRegistryStats {
        let guard = self.entries.read();
        let mut stats = PluginRegistryStats::default();
        stats.total_registry_entries = guard.len();
        stats.registry_path = self
            .source_path
            .as_ref()
            .map(|path| path.display().to_string());
        let mut last_reviewed = None;
        for record in guard.values() {
            match record.status.as_deref() {
                Some(status) if status.eq_ignore_ascii_case("active") => stats.active_plugins += 1,
                Some(status) if status.eq_ignore_ascii_case("pending") => stats.pending_review += 1,
                _ => {}
            }
            if let Some(ts) = &record.last_reviewed_at {
                if last_reviewed.as_deref() < Some(ts.as_str()) {
                    last_reviewed = Some(ts.clone());
                }
            }
        }
        stats.last_reviewed_at = last_reviewed;
        stats
    }

    pub fn entries(&self) -> Vec<PluginRecord> {
        self.entries.read().values().cloned().collect()
    }

    pub fn get(&self, id: &str) -> Option<PluginRecord> {
        self.entries.read().get(id).cloned()
    }

    pub fn update_status<S: Into<String>>(&self, id: &str, status: S) -> Option<PluginRecord> {
        let mut guard = self.entries.write();
        if let Some(record) = guard.get_mut(id) {
            record.status = Some(status.into());
            record.last_reviewed_at = Some(Utc::now().to_rfc3339());
            return Some(record.clone());
        }
        None
    }

    pub fn update_plugin<F>(&self, id: &str, update: F) -> Result<PluginRecord, RegistryError>
    where
        F: FnOnce(&mut PluginRecord),
    {
        let mut guard = self.entries.write();
        let record = guard
            .get_mut(id)
            .ok_or_else(|| RegistryError::PluginNotFound(id.to_string()))?;
        update(record);
        Ok(record.clone())
    }

    pub fn save(&self) -> std::io::Result<()> {
        let Some(path) = &self.source_path else {
            return Ok(());
        };
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut plugins = self.entries.read().values().cloned().collect::<Vec<_>>();
        plugins.sort_by(|a, b| a.id.cmp(&b.id));
        let file = PluginRegistryFile { plugins };
        let payload = serde_json::to_string_pretty(&file)?;
        fs::write(path, payload)
    }

    pub fn source_path(&self) -> Option<&Path> {
        self.source_path.as_deref()
    }

    pub fn helpers_for_targets(&self, targets: &[String]) -> Vec<RegistryHelper> {
        if targets.is_empty() {
            return Vec::new();
        }
        let guard = self.entries.read();
        let mut helpers = Vec::new();
        for record in guard.values() {
            if !record
                .status
                .as_deref()
                .map(|status| status.eq_ignore_ascii_case("active"))
                .unwrap_or(true)
            {
                continue;
            }
            for helper in &record.helpers {
                if helper.matches_any(targets) {
                    helpers.push(helper.clone_with_plugin(&record.id));
                }
            }
        }
        helpers
    }

    pub fn plugin_helpers(&self, plugin_id: &str) -> Option<Vec<RegistryHelper>> {
        self.entries
            .read()
            .get(plugin_id)
            .map(|record| record.helpers.clone())
    }

    pub fn add_helper(
        &self,
        plugin_id: &str,
        mut helper: RegistryHelper,
    ) -> Result<RegistryHelper, RegistryError> {
        let mut guard = self.entries.write();
        let record = guard
            .get_mut(plugin_id)
            .ok_or_else(|| RegistryError::PluginNotFound(plugin_id.to_string()))?;
        helper.normalize(plugin_id);
        if helper.resolved_steps().is_empty() {
            return Err(RegistryError::HelperMissingSteps(helper.id.clone()));
        }
        if record
            .helpers
            .iter()
            .any(|existing| existing.id == helper.id)
        {
            return Err(RegistryError::HelperExists(helper.id.clone()));
        }
        record.helpers.push(helper.clone());
        Ok(helper)
    }

    pub fn update_helper(
        &self,
        plugin_id: &str,
        helper_id: &str,
        mut helper: RegistryHelper,
    ) -> Result<RegistryHelper, RegistryError> {
        let mut guard = self.entries.write();
        let record = guard
            .get_mut(plugin_id)
            .ok_or_else(|| RegistryError::PluginNotFound(plugin_id.to_string()))?;
        helper.id = helper_id.to_string();
        helper.normalize(plugin_id);
        if helper.resolved_steps().is_empty() {
            return Err(RegistryError::HelperMissingSteps(helper.id.clone()));
        }
        if let Some(existing) = record
            .helpers
            .iter_mut()
            .find(|existing| existing.id == helper.id)
        {
            *existing = helper.clone();
            return Ok(helper);
        }
        Err(RegistryError::HelperNotFound(helper_id.to_string()))
    }

    pub fn delete_helper(&self, plugin_id: &str, helper_id: &str) -> Result<(), RegistryError> {
        let mut guard = self.entries.write();
        let record = guard
            .get_mut(plugin_id)
            .ok_or_else(|| RegistryError::PluginNotFound(plugin_id.to_string()))?;
        let original_len = record.helpers.len();
        record.helpers.retain(|helper| helper.id != helper_id);
        if record.helpers.len() == original_len {
            return Err(RegistryError::HelperNotFound(helper_id.to_string()));
        }
        Ok(())
    }
}

#[cfg(test)]
impl PluginRegistry {
    pub fn from_records_for_tests(records: Vec<PluginRecord>) -> Self {
        Self::from_records(records, None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helpers_match_targets() {
        let helper = RegistryHelper {
            id: "helper".into(),
            pattern: "example\\.com".into(),
            description: Some("accept banner".into()),
            blockers: Vec::new(),
            auto_insert: true,
            prompt: None,
            step: None,
            steps: vec![HelperStep {
                title: "Accept banner".into(),
                detail: Some("click consent".into()),
                wait: Some("dom_ready".into()),
                timeout_ms: Some(5000),
                tool: HelperTool::ClickCss {
                    selector: "#accept".into(),
                },
            }],
            conditions: HelperConditions::default(),
            plugin_id: None,
        };
        let record = PluginRecord {
            id: "demo".into(),
            status: Some("active".into()),
            owner: None,
            last_reviewed_at: None,
            description: None,
            scopes: None,
            helpers: vec![helper],
        };
        let registry = PluginRegistry::from_records_for_tests(vec![record]);
        let helpers = registry.helpers_for_targets(&["https://www.example.com".into()]);
        assert_eq!(helpers.len(), 1);
        assert_eq!(helpers[0].plugin_id(), Some("demo"));
    }
}

#[derive(Deserialize, Serialize)]
struct PluginRegistryFile {
    plugins: Vec<PluginRecord>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RegistryHelper {
    pub id: String,
    pub pattern: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub blockers: Vec<String>,
    #[serde(default)]
    pub auto_insert: bool,
    #[serde(default)]
    pub prompt: Option<String>,
    #[serde(default)]
    pub step: Option<HelperStep>,
    #[serde(default)]
    pub steps: Vec<HelperStep>,
    #[serde(default)]
    pub conditions: HelperConditions,
    #[serde(skip)]
    pub(crate) plugin_id: Option<String>,
}

impl RegistryHelper {
    fn matches_any(&self, targets: &[String]) -> bool {
        targets.iter().any(|target| self.matches(target))
    }

    fn matches(&self, target: &str) -> bool {
        if target.is_empty() {
            return false;
        }
        let pattern_match = match Regex::new(&self.pattern) {
            Ok(regex) => regex.is_match(target),
            Err(err) => {
                warn!(
                    pattern = %self.pattern,
                    ?err,
                    "invalid registry helper pattern; falling back to substring"
                );
                target.contains(&self.pattern)
            }
        };
        if !pattern_match {
            return false;
        }
        if !self.conditions.url_includes.is_empty()
            && !self
                .conditions
                .url_includes
                .iter()
                .any(|needle| !needle.is_empty() && target.contains(needle))
        {
            return false;
        }
        if self
            .conditions
            .url_excludes
            .iter()
            .any(|needle| !needle.is_empty() && target.contains(needle))
        {
            return false;
        }
        true
    }

    pub fn plugin_id(&self) -> Option<&str> {
        self.plugin_id.as_deref()
    }

    fn clone_with_plugin(&self, plugin_id: &str) -> Self {
        let mut cloned = self.clone();
        cloned.plugin_id = Some(plugin_id.to_string());
        cloned
    }

    pub fn matches_target(&self, target: &str) -> bool {
        self.matches(target)
    }

    fn normalize(&mut self, plugin_id: &str) {
        self.plugin_id = Some(plugin_id.to_string());
        if self.steps.is_empty() {
            if let Some(step) = self.step.take() {
                self.steps.push(step);
            }
        }
        if self.steps.is_empty() {
            warn!(helper = %self.id, plugin = plugin_id, "registry helper missing steps");
        }
    }

    pub fn resolved_steps(&self) -> &[HelperStep] {
        &self.steps
    }

    pub fn matches_blocker(&self, blocker: Option<&str>) -> bool {
        if self.blockers.is_empty() {
            return true;
        }
        blocker
            .map(|value| self.blockers.iter().any(|candidate| candidate == value))
            .unwrap_or(false)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HelperStep {
    pub title: String,
    #[serde(default)]
    pub detail: Option<String>,
    #[serde(default)]
    pub wait: Option<String>,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    pub tool: HelperTool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HelperTool {
    ClickCss {
        selector: String,
    },
    ClickText {
        text: String,
        #[serde(default)]
        exact: bool,
    },
    Custom {
        name: String,
        #[serde(default)]
        payload: Value,
    },
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct HelperConditions {
    #[serde(default)]
    pub url_includes: Vec<String>,
    #[serde(default)]
    pub url_excludes: Vec<String>,
}
