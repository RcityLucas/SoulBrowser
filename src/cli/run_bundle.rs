use std::path::Path;

use anyhow::{Context, Result};
use serde_json::Value;
use tokio::fs;

pub async fn load_run_bundle(path: impl AsRef<Path>) -> Result<Value> {
    let path = path.as_ref();
    let content = fs::read_to_string(path)
        .await
        .with_context(|| format!("failed to read run bundle {}", path.display()))?;
    let bundle: Value = serde_json::from_str(&content)
        .with_context(|| format!("failed to parse run bundle {}", path.display()))?;
    Ok(bundle)
}
