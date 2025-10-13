//! Policy configuration loader for browser routes.
//!
//! Provides a thin wrapper around the soul-base route policy definitions so
//! higher layers can load policy specs from disk (or fall back to defaults)
//! and feed them into the interceptor chain and auth manager.

use crate::errors::SoulBrowserError;
use serde_json::{json, Value};
use soulbase_interceptors::policy::{
    dsl::RoutePolicy,
    model::{MatchCond, RouteBindingSpec, RoutePolicySpec},
};
use std::path::{Path, PathBuf};
use tokio::fs;

/// Wrapper over route policies used by the browser runtime.
pub struct BrowserPolicy {
    specs: Vec<RoutePolicySpec>,
}

impl BrowserPolicy {
    /// Load policy specs from `SOUL_POLICY_PATH` or fall back to built-in defaults.
    #[allow(dead_code)]
    pub async fn load() -> Result<Self, SoulBrowserError> {
        let empty: Vec<PathBuf> = Vec::new();
        Self::load_with_paths(&empty).await
    }

    pub async fn load_with_paths(additional_paths: &[PathBuf]) -> Result<Self, SoulBrowserError> {
        if let Ok(path) = std::env::var("SOUL_POLICY_PATH") {
            if let Some(specs) = Self::load_from_path(Path::new(&path)).await? {
                return Ok(Self { specs });
            }
        }

        for path in additional_paths {
            if let Some(specs) = Self::load_from_path(path.as_path()).await? {
                return Ok(Self { specs });
            }
        }

        for rel in DEFAULT_POLICY_PATHS {
            let path = Path::new(rel);
            if let Some(specs) = Self::load_from_path(path).await? {
                return Ok(Self { specs });
            }
        }

        Ok(Self {
            specs: default_specs(),
        })
    }

    /// Get the route policy DSL representation.
    pub fn route_policy(&self) -> RoutePolicy {
        RoutePolicy::new(self.specs.clone())
    }

    /// Access the raw specs (useful for debugging or exporting).
    #[allow(dead_code)]
    pub fn specs(&self) -> &[RoutePolicySpec] {
        &self.specs
    }
}

fn default_specs() -> Vec<RoutePolicySpec> {
    vec![
        RoutePolicySpec {
            when: MatchCond::Http {
                method: "POST".into(),
                path_glob: "browser://session/navigate".into(),
            },
            bind: RouteBindingSpec {
                resource: "browser:session:navigate".into(),
                action: "Invoke".into(),
                attrs_template: Some(json!({
                    "operation": "navigate",
                    "channel": "automation",
                })),
                attrs_from_body: true,
            },
        },
        RoutePolicySpec {
            when: MatchCond::Http {
                method: "POST".into(),
                path_glob: "browser://session/click".into(),
            },
            bind: RouteBindingSpec {
                resource: "browser:session:click".into(),
                action: "Invoke".into(),
                attrs_template: Some(json!({
                    "operation": "click",
                    "channel": "automation",
                })),
                attrs_from_body: true,
            },
        },
        RoutePolicySpec {
            when: MatchCond::Http {
                method: "POST".into(),
                path_glob: "browser://session/type".into(),
            },
            bind: RouteBindingSpec {
                resource: "browser:session:type".into(),
                action: "Invoke".into(),
                attrs_template: Some(json!({
                    "operation": "type",
                    "channel": "automation",
                })),
                attrs_from_body: true,
            },
        },
        RoutePolicySpec {
            when: MatchCond::Http {
                method: "POST".into(),
                path_glob: "browser://session/screenshot".into(),
            },
            bind: RouteBindingSpec {
                resource: "browser:session:screenshot".into(),
                action: "Read".into(),
                attrs_template: Some(json!({
                    "operation": "screenshot",
                    "channel": "automation",
                })),
                attrs_from_body: false,
            },
        },
    ]
}

fn resolve_path(path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    }
}

impl BrowserPolicy {
    async fn load_from_path(path: &Path) -> Result<Option<Vec<RoutePolicySpec>>, SoulBrowserError> {
        let resolved = resolve_path(path);
        if !fs::try_exists(&resolved).await.unwrap_or(false) {
            return Ok(None);
        }

        let content = fs::read_to_string(&resolved).await?;
        if content.trim().is_empty() {
            return Ok(None);
        }

        let specs: Vec<RoutePolicySpec> = serde_json::from_str(&content)?;
        tracing::info!(
            path = %resolved.display(),
            count = specs.len(),
            "Loaded browser policy specifications"
        );
        Ok(Some(specs))
    }
}

/// Merge two JSON objects recursively (used to enrich policy attributes).
pub fn merge_attrs(base: &mut Value, extra: Value) {
    match (base, extra) {
        (Value::Object(base_map), Value::Object(extra_map)) => {
            for (k, v) in extra_map {
                merge_attrs(base_map.entry(k).or_insert(Value::Null), v);
            }
        }
        (slot, value) => {
            *slot = value;
        }
    }
}

// Ensure the default specs remain serializable for config dumps.
#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    #[tokio::test]
    async fn default_policy_loads() {
        let policy = BrowserPolicy::load().await.expect("load policy");
        assert!(!policy.specs.is_empty());
        let serialized = serde_json::to_string(policy.specs()).expect("serialize specs");
        assert!(serialized.contains("browser:session:navigate"));
    }

    #[tokio::test]
    async fn load_policy_from_custom_path() {
        let file = NamedTempFile::new().expect("temp policy file");
        let policy_spec = serde_json::json!([
            {
                "when": { "Http": { "method": "POST", "path_glob": "browser://session/custom" } },
                "bind": {
                    "resource": "browser:session:custom",
                    "action": "Invoke",
                    "attrs_template": { "operation": "custom" }
                }
            }
        ]);
        std::fs::write(file.path(), serde_json::to_string(&policy_spec).unwrap()).unwrap();

        let paths = vec![file.path().to_path_buf()];
        let policy = BrowserPolicy::load_with_paths(&paths)
            .await
            .expect("load custom policy");
        let route_policy = policy.route_policy();
        assert!(route_policy
            .match_http("POST", "browser://session/custom")
            .is_some());
    }
}
const DEFAULT_POLICY_PATHS: &[&str] = &[
    "config/policies/browser_policy.json",
    "config/browser_policy.json",
];
