use anyhow::{anyhow, Result};
use chromiumoxide::async_process::Child;
use futures::io::{AsyncBufReadExt, BufReader};
use futures::stream::StreamExt;
use tokio::time::{timeout, Duration};

/// Extract DevTools websocket URL from Chromium stderr output.
pub async fn extract_ws_url(child: &mut Child) -> Result<String> {
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| anyhow!("chromium process missing stderr handle"))?;
    let mut lines = BufReader::new(stderr).lines();
    let mut captured = Vec::new();

    let reader = async {
        while let Some(line) = lines.next().await {
            let line = line?;
            captured.push(line.clone());
            if let Some((_, ws)) = line.rsplit_once("listening on ") {
                let ws = ws.trim();
                if ws.starts_with("ws") && ws.contains("devtools/browser") {
                    return Ok(ws.to_string());
                }
            }
        }
        Err(anyhow!(
            "chromium exited before exposing devtools websocket url. stderr preview: {}",
            captured
                .iter()
                .take(8)
                .cloned()
                .collect::<Vec<_>>()
                .join(" | ")
        ))
    };

    timeout(Duration::from_secs(20), reader)
        .await
        .map_err(|_| anyhow!("timed out waiting for chromium devtools websocket url"))?
}
