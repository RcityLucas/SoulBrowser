use std::path::PathBuf;

use anyhow::{Context, Result};
use base64::{engine::general_purpose::STANDARD as Base64, Engine as _};
use perceiver_hub::models::MultiModalPerception;
use serde::Deserialize;
use tokio::fs;

#[derive(Clone, Debug)]
pub struct PerceptionExecResult {
    pub(crate) success: bool,
    pub(crate) perception: Option<MultiModalPerception>,
    pub(crate) screenshot_base64: Option<String>,
    pub(crate) stdout: String,
    pub(crate) stderr: String,
    pub(crate) error_message: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ConsoleFixture {
    #[serde(default)]
    success: Option<bool>,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    stdout: Option<String>,
    #[serde(default)]
    stderr: Option<String>,
    #[serde(default)]
    perception: Option<MultiModalPerception>,
    #[serde(default)]
    screenshot_base64: Option<String>,
}

pub async fn load_console_fixture() -> Result<Option<PerceptionExecResult>> {
    let path = match std::env::var("SOULBROWSER_CONSOLE_FIXTURE") {
        Ok(path) if !path.trim().is_empty() => PathBuf::from(path),
        Ok(_) | Err(std::env::VarError::NotPresent) => return Ok(None),
        Err(err) => {
            return Err(anyhow::anyhow!(
                "failed to read SOULBROWSER_CONSOLE_FIXTURE env var: {}",
                err
            ));
        }
    };

    let data = fs::read(&path)
        .await
        .with_context(|| format!("failed to read console fixture {}", path.display()))?;

    let fixture: ConsoleFixture = serde_json::from_slice(&data)
        .with_context(|| format!("failed to parse console fixture {}", path.display()))?;

    let success = fixture.success.unwrap_or(true);
    let perception = if success {
        Some(
            fixture
                .perception
                .ok_or_else(|| anyhow::anyhow!("console fixture missing 'perception' payload"))?,
        )
    } else {
        fixture.perception
    };

    let screenshot_base64 = match std::env::var("SOULBROWSER_CONSOLE_FIXTURE_SCREENSHOT") {
        Ok(extra_path) if !extra_path.trim().is_empty() => {
            let img_path = PathBuf::from(extra_path);
            let bytes = fs::read(&img_path).await.with_context(|| {
                format!(
                    "failed to read console fixture screenshot {}",
                    img_path.display()
                )
            })?;
            Some(Base64.encode(bytes))
        }
        Ok(_) | Err(std::env::VarError::NotPresent) => fixture.screenshot_base64,
        Err(err) => {
            return Err(anyhow::anyhow!(
                "failed to read SOULBROWSER_CONSOLE_FIXTURE_SCREENSHOT env var: {}",
                err
            ));
        }
    };

    let stdout = fixture
        .stdout
        .unwrap_or_else(|| "console fixture: perception executed".to_string());
    let stderr = fixture.stderr.unwrap_or_default();

    Ok(Some(PerceptionExecResult {
        success,
        perception,
        screenshot_base64,
        stdout,
        stderr,
        error_message: fixture.error,
    }))
}
