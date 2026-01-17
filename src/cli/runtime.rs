use std::env;
use std::fs as stdfs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use dirs;
use serde_yaml;
use soulbrowser_kernel::Config;
use tokio::fs;
use tracing::{info, warn};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

pub fn load_local_env_overrides() {
    let path = Path::new("config/local.env");
    if !path.exists() {
        return;
    }

    match stdfs::read_to_string(path) {
        Ok(contents) => {
            for (idx, raw_line) in contents.lines().enumerate() {
                let line = raw_line.trim();
                if line.is_empty() || line.starts_with('#') {
                    continue;
                }
                let Some((key, value)) = line.split_once('=') else {
                    warn!(line = idx + 1, "invalid local.env entry; skipping");
                    continue;
                };
                let key = key.trim();
                if key.is_empty() || env::var(key).is_ok() {
                    continue;
                }
                let normalized = unescape_value(value.trim());
                env::set_var(key, normalized);
            }
            info!(path = %path.display(), "Loaded environment overrides from local.env");
        }
        Err(err) => {
            warn!(path = %path.display(), ?err, "failed to read local.env overrides");
        }
    }
}

pub fn init_logging(level: &str, debug: bool) -> Result<()> {
    let level = if debug {
        tracing::Level::DEBUG
    } else {
        level.parse().context("Invalid log level")?
    };

    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(level.to_string())),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    Ok(())
}

pub struct LoadedConfig {
    pub config: Config,
    pub path: PathBuf,
}

pub async fn load_config(config_path: Option<&PathBuf>) -> Result<LoadedConfig> {
    let config_path = match config_path {
        Some(path) => path.clone(),
        None => {
            // Priority: ./config/config.yaml > ~/.config/soulbrowser/config.yaml
            let local_config = PathBuf::from("config/config.yaml");
            if local_config.exists() {
                local_config
            } else {
                let mut path = dirs::config_dir().context("Failed to get config directory")?;
                path.push("soulbrowser");
                path.push("config.yaml");
                path
            }
        }
    };

    if config_path.exists() {
        let content = fs::read_to_string(&config_path)
            .await
            .context("Failed to read config file")?;

        let config: Config =
            serde_yaml::from_str(&content).context("Failed to parse config file")?;

        info!("Loaded configuration from: {}", config_path.display());
        Ok(LoadedConfig {
            config,
            path: config_path,
        })
    } else {
        warn!(
            "Config file not found, using defaults: {}",
            config_path.display()
        );
        Ok(LoadedConfig {
            config: Config::default(),
            path: config_path,
        })
    }
}

pub fn apply_runtime_overrides(config: &Config) {
    if config.strict_authorization && env::var("SOUL_STRICT_AUTHZ").is_err() {
        env::set_var("SOUL_STRICT_AUTHZ", "true");
        info!("Enabled strict authorization (SOUL_STRICT_AUTHZ=true)");
    }

    if env::var("SOUL_POLICY_PATH").is_err() {
        if let Some(path) = config.policy_paths.first() {
            env::set_var("SOUL_POLICY_PATH", path);
            info!("Using policy file from config: {}", path.display());
        }
    }
}

fn unescape_value(value: &str) -> String {
    if value.starts_with('"') && value.ends_with('"') && value.len() >= 2 {
        let inner = &value[1..value.len() - 1];
        inner
            .replace("\\\"", "\"")
            .replace("\\n", "\n")
            .replace("\\r", "\r")
            .replace("\\t", "\t")
    } else {
        value.to_string()
    }
}
