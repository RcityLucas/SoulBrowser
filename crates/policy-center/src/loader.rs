use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::api::apply_override_to_snapshot;
use crate::defaults::default_snapshot;
use crate::errors::PolicyError;
use crate::model::{PolicySnapshot, PolicySource};

const ENV_PREFIX: &str = "SOUL_POLICY__";
const ENV_JSON: &str = "SOUL_POLICY_OVERRIDE_JSON";
const ENV_CLI_OVERRIDES: &str = "SOUL_POLICY_CLI_OVERRIDES";

#[derive(Debug, Default)]
pub struct LoadOptions {
    pub paths: Vec<PathBuf>,
    pub include_env: bool,
    pub include_cli_env: bool,
}

impl LoadOptions {
    pub fn with_path(path: impl Into<PathBuf>) -> Self {
        Self {
            paths: vec![path.into()],
            include_env: true,
            include_cli_env: true,
        }
    }
}

pub fn load_snapshot(path: Option<&Path>) -> Result<PolicySnapshot, PolicyError> {
    let mut options = LoadOptions::default();
    if let Some(p) = path {
        options.paths.push(p.to_path_buf());
    }
    options.include_env = true;
    options.include_cli_env = true;
    load_snapshot_with_options(&options)
}

pub fn load_snapshot_with_options(options: &LoadOptions) -> Result<PolicySnapshot, PolicyError> {
    let mut snapshot = default_snapshot();
    bootstrap_builtin_provenance(&mut snapshot)?;

    for path in &options.paths {
        if path.exists() {
            let overlay = overlays_from_file(path)?;
            apply_overlays(&mut snapshot, overlay)?;
        }
    }

    if options.include_env {
        let env_overlays = overlays_from_env()?;
        apply_overlays(&mut snapshot, env_overlays)?;
    }

    if options.include_cli_env {
        let cli_overlays = overlays_from_cli_env()?;
        apply_overlays(&mut snapshot, cli_overlays)?;
    }

    Ok(snapshot)
}

struct PolicyOverlay {
    path: String,
    value: Value,
    source: PolicySource,
}

fn apply_overlays(
    snapshot: &mut PolicySnapshot,
    overlays: Vec<PolicyOverlay>,
) -> Result<(), PolicyError> {
    for overlay in overlays {
        apply_override_to_snapshot(snapshot, &overlay.path, &overlay.value, overlay.source)?;
    }
    Ok(())
}

fn overlays_from_file(path: &Path) -> Result<Vec<PolicyOverlay>, PolicyError> {
    let content = fs::read_to_string(path).map_err(|err| PolicyError::Io(format!("{}", err)))?;
    let yaml_value: serde_yaml::Value =
        serde_yaml::from_str(&content).map_err(|err| PolicyError::Invalid(format!("{}", err)))?;
    let json_value =
        serde_json::to_value(yaml_value).map_err(|err| PolicyError::Invalid(format!("{}", err)))?;
    Ok(flatten_value(json_value, None, PolicySource::File))
}

fn overlays_from_env() -> Result<Vec<PolicyOverlay>, PolicyError> {
    let mut overlays = Vec::new();
    for (key, raw) in env::vars() {
        if let Some(stripped) = key.strip_prefix(ENV_PREFIX) {
            let path = stripped
                .split("__")
                .filter(|segment| !segment.is_empty())
                .map(|segment| segment.to_ascii_lowercase())
                .collect::<Vec<_>>()
                .join(".");
            if path.is_empty() {
                continue;
            }
            let value = parse_env_value(&raw);
            overlays.push(PolicyOverlay {
                path,
                value,
                source: PolicySource::Env,
            });
        }
    }

    if let Ok(raw_json) = env::var(ENV_JSON) {
        if !raw_json.trim().is_empty() {
            let json_value: Value = serde_json::from_str(&raw_json)
                .map_err(|err| PolicyError::Invalid(format!("{}", err)))?;
            overlays.extend(flatten_value(json_value, None, PolicySource::Env));
        }
    }

    Ok(overlays)
}

fn overlays_from_cli_env() -> Result<Vec<PolicyOverlay>, PolicyError> {
    let mut overlays = Vec::new();
    if let Ok(raw) = env::var(ENV_CLI_OVERRIDES) {
        for token in raw.split(',') {
            let trimmed = token.trim();
            if trimmed.is_empty() {
                continue;
            }
            let mut parts = trimmed.splitn(2, '=');
            let path = parts.next().unwrap().trim();
            let value_raw = parts.next().unwrap_or("").trim();
            if path.is_empty() {
                continue;
            }
            let value = parse_env_value(value_raw);
            overlays.push(PolicyOverlay {
                path: path.to_string(),
                value,
                source: PolicySource::Cli,
            });
        }
    }
    Ok(overlays)
}

fn parse_env_value(raw: &str) -> Value {
    if raw.is_empty() {
        return Value::Null;
    }
    if let Ok(parsed) = serde_json::from_str::<Value>(raw) {
        return parsed;
    }
    if let Ok(boolean) = raw.parse::<bool>() {
        return Value::Bool(boolean);
    }
    if let Ok(int_val) = raw.parse::<i64>() {
        return Value::Number(int_val.into());
    }
    Value::String(raw.to_string())
}

fn flatten_value(value: Value, prefix: Option<String>, source: PolicySource) -> Vec<PolicyOverlay> {
    match value {
        Value::Object(map) => {
            let mut result = Vec::new();
            for (key, value) in map {
                let key_segment = key.trim().to_ascii_lowercase();
                let next_prefix = match &prefix {
                    Some(prefix) if !prefix.is_empty() => format!("{}.{}", prefix, key_segment),
                    Some(_) => key_segment.clone(),
                    None => key_segment.clone(),
                };
                result.extend(flatten_value(value, Some(next_prefix), source));
            }
            result
        }
        other => {
            if let Some(prefix) = prefix {
                vec![PolicyOverlay {
                    path: prefix,
                    value: other,
                    source,
                }]
            } else {
                Vec::new()
            }
        }
    }
}

fn bootstrap_builtin_provenance(snapshot: &mut PolicySnapshot) -> Result<(), PolicyError> {
    let mut overlays = Vec::new();
    overlays.extend(flatten_value(
        serde_json::to_value(&snapshot.scheduler)
            .map_err(|err| PolicyError::Invalid(format!("{}", err)))?,
        Some("scheduler".into()),
        PolicySource::Builtin,
    ));
    overlays.extend(flatten_value(
        serde_json::to_value(&snapshot.registry)
            .map_err(|err| PolicyError::Invalid(format!("{}", err)))?,
        Some("registry".into()),
        PolicySource::Builtin,
    ));
    overlays.extend(flatten_value(
        serde_json::to_value(&snapshot.features)
            .map_err(|err| PolicyError::Invalid(format!("{}", err)))?,
        Some("features".into()),
        PolicySource::Builtin,
    ));

    for overlay in overlays {
        snapshot.set_provenance(&overlay.path, overlay.source);
    }
    Ok(())
}
