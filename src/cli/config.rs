use std::path::Path;

use crate::cli::context::CliContext;
use anyhow::{bail, Context, Result};
use clap::{Args, Subcommand};
use serde_json::{Map, Value as JsonValue};
use serde_yaml;
use soulbrowser_kernel::Config;
use tokio::fs;
use tracing::info;

#[derive(Args, Clone, Debug)]
pub struct ConfigArgs {
    #[command(subcommand)]
    pub action: ConfigAction,
}

#[derive(Subcommand, Clone, Debug)]
pub enum ConfigAction {
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

pub async fn cmd_config(args: ConfigArgs, ctx: &CliContext) -> Result<()> {
    let path = ctx.config_path().to_path_buf();
    match args.action {
        ConfigAction::Show => {
            let config = load_config_file(&path).await?;
            println!("Current configuration ({}):", path.display());
            println!("{}", serde_yaml::to_string(&config)?);
        }
        ConfigAction::Set { key, value } => {
            let mut config = load_config_file(&path).await?;
            let mut json = serde_json::to_value(&config)?;
            let segments = split_key(&key)?;
            let parsed = parse_cli_value(&value);
            set_json_value(&mut json, &segments, parsed)?;
            config = serde_json::from_value(json)?;
            save_config_file(&path, &config).await?;
            info!("Updated configuration key {}", key);
            println!("Saved configuration to {}", path.display());
        }
        ConfigAction::Get { key } => {
            let config = load_config_file(&path).await?;
            let json = serde_json::to_value(&config)?;
            let segments = split_key(&key)?;
            if let Some(value) = get_json_value(&json, &segments) {
                println!("{}", serde_yaml::to_string(value)?);
            } else {
                bail!("{} not found in configuration", key);
            }
        }
        ConfigAction::Reset => {
            let defaults = Config::default();
            save_config_file(&path, &defaults).await?;
            println!(
                "Configuration reset to defaults and written to {}",
                path.display()
            );
        }
        ConfigAction::Validate => {
            if fs::try_exists(&path).await? {
                let raw = fs::read_to_string(&path)
                    .await
                    .with_context(|| format!("reading {}", path.display()))?;
                serde_yaml::from_str::<Config>(&raw)
                    .with_context(|| format!("parsing {}", path.display()))?;
                println!("Configuration file {} is valid", path.display());
            } else {
                println!(
                    "No configuration file at {}; defaults are valid",
                    path.display()
                );
            }
        }
    }

    Ok(())
}

async fn load_config_file(path: &Path) -> Result<Config> {
    if fs::try_exists(path).await? {
        let raw = fs::read_to_string(path)
            .await
            .with_context(|| format!("reading {}", path.display()))?;
        let config =
            serde_yaml::from_str(&raw).with_context(|| format!("parsing {}", path.display()))?;
        Ok(config)
    } else {
        Ok(Config::default())
    }
}

async fn save_config_file(path: &Path, config: &Config) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .await
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    let serialized = serde_yaml::to_string(config)?;
    fs::write(path, serialized)
        .await
        .with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

fn parse_cli_value(raw: &str) -> JsonValue {
    serde_json::from_str(raw).unwrap_or_else(|_| JsonValue::String(raw.to_string()))
}

fn split_key(key: &str) -> Result<Vec<&str>> {
    let segments: Vec<&str> = key
        .split('.')
        .filter(|segment| !segment.is_empty())
        .collect();
    if segments.is_empty() {
        bail!("configuration key cannot be empty");
    }
    Ok(segments)
}

fn set_json_value(target: &mut JsonValue, path: &[&str], value: JsonValue) -> Result<()> {
    if path.is_empty() {
        bail!("configuration key cannot be empty");
    }
    let mut current = target;
    for segment in &path[..path.len() - 1] {
        current = {
            let map = ensure_object(current, segment)?;
            map.entry((*segment).to_string()).or_insert(JsonValue::Null)
        };
    }
    let map = ensure_object(current, path.last().unwrap())?;
    map.insert(path.last().unwrap().to_string(), value);
    Ok(())
}

fn ensure_object<'a>(
    value: &'a mut JsonValue,
    segment: &str,
) -> Result<&'a mut Map<String, JsonValue>> {
    if !value.is_object() {
        if value.is_null() {
            *value = JsonValue::Object(Map::new());
        } else {
            bail!(
                "{} resolves to a non-object value; cannot assign nested configuration",
                segment
            );
        }
    }
    Ok(value
        .as_object_mut()
        .expect("value must be an object after normalization"))
}

fn get_json_value<'a>(value: &'a JsonValue, path: &[&str]) -> Option<&'a JsonValue> {
    let mut current = value;
    for segment in path {
        match current {
            JsonValue::Object(map) => {
                current = map.get(*segment)?;
            }
            _ => return None,
        }
    }
    Some(current)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn set_and_get_nested_keys() {
        let mut doc = json!({});
        set_json_value(&mut doc, &["soul", "enabled"], JsonValue::Bool(true)).unwrap();
        set_json_value(
            &mut doc,
            &["performance", "thresholds", "page_load_time"],
            JsonValue::from(4200),
        )
        .unwrap();
        assert_eq!(
            get_json_value(&doc, &["soul", "enabled"]),
            Some(&JsonValue::Bool(true))
        );
        assert_eq!(
            get_json_value(&doc, &["performance", "thresholds", "page_load_time"]),
            Some(&JsonValue::from(4200))
        );
    }
}
