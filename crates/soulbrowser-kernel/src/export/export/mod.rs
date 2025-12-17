//! Data export module
//!
//! Provides various exporters for session data

use anyhow::{Context, Result};
use csv;
use serde::Serialize;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;

use crate::{
    app_context::AppContext,
    storage::{QueryParams, StorageManager},
};

/// Base trait for data exporters
#[async_trait::async_trait]
pub trait Exporter: Send + Sync {
    async fn export(&self, output_path: &PathBuf) -> Result<ExportStats>;
}

/// Statistics from export operations
#[derive(Debug, Serialize)]
pub struct ExportStats {
    pub total_events: usize,
    pub total_sessions: usize,
    pub bytes_written: u64,
    pub duration_ms: u64,
}

/// Export events as JSON
pub struct JsonExporter {
    storage: Arc<StorageManager>,
    session_id: Option<String>,
}

impl JsonExporter {
    pub fn with_context(context: Arc<AppContext>, session_id: Option<String>) -> Self {
        Self {
            storage: context.storage(),
            session_id,
        }
    }
}

#[async_trait::async_trait]
impl Exporter for JsonExporter {
    async fn export(&self, output_path: &PathBuf) -> Result<ExportStats> {
        let start = std::time::Instant::now();

        // Query all events
        let query = QueryParams {
            session_id: self.session_id.clone(),
            event_type: None,
            from_timestamp: None,
            to_timestamp: None,
            limit: 10000,
            offset: 0,
        };

        let events = self
            .storage
            .backend()
            .query_events(query)
            .await
            .context("Failed to query events")?;

        // Serialize to JSON
        let json = serde_json::to_string_pretty(&events).context("Failed to serialize events")?;

        // Write to file
        let mut file =
            std::fs::File::create(output_path).context("Failed to create output file")?;
        file.write_all(json.as_bytes())
            .context("Failed to write JSON")?;

        Ok(ExportStats {
            total_events: events.len(),
            total_sessions: 1,
            bytes_written: json.len() as u64,
            duration_ms: start.elapsed().as_millis() as u64,
        })
    }
}

/// Export events as CSV
pub struct CsvExporter {
    storage: Arc<StorageManager>,
    session_id: Option<String>,
}

impl CsvExporter {
    pub fn with_context(context: Arc<AppContext>, session_id: Option<String>) -> Self {
        Self {
            storage: context.storage(),
            session_id,
        }
    }
}

#[async_trait::async_trait]
impl Exporter for CsvExporter {
    async fn export(&self, output_path: &PathBuf) -> Result<ExportStats> {
        let start = std::time::Instant::now();

        // Query all events
        let query = QueryParams {
            session_id: self.session_id.clone(),
            event_type: None,
            from_timestamp: None,
            to_timestamp: None,
            limit: 10000,
            offset: 0,
        };

        let events = self
            .storage
            .backend()
            .query_events(query)
            .await
            .context("Failed to query events")?;

        // Create CSV writer
        let mut wtr = csv::Writer::from_path(output_path).context("Failed to create CSV writer")?;

        // Write header
        wtr.write_record(&["id", "session_id", "timestamp", "event_type", "data"])
            .context("Failed to write CSV header")?;

        // Write events
        let mut bytes_written = 0u64;
        for event in &events {
            let data_str = serde_json::to_string(&event.data).unwrap_or_else(|_| "{}".to_string());

            wtr.write_record(&[
                &event.id,
                &event.session_id,
                &event.timestamp.to_string(),
                &event.event_type,
                &data_str,
            ])
            .context("Failed to write CSV record")?;

            bytes_written += data_str.len() as u64;
        }

        wtr.flush().context("Failed to flush CSV writer")?;

        Ok(ExportStats {
            total_events: events.len(),
            total_sessions: 1,
            bytes_written,
            duration_ms: start.elapsed().as_millis() as u64,
        })
    }
}

/// Export events and analytics as HTML report
pub struct HtmlExporter {
    storage: Arc<StorageManager>,
    session_id: Option<String>,
}

impl HtmlExporter {
    pub fn with_context(context: Arc<AppContext>, session_id: Option<String>) -> Self {
        Self {
            storage: context.storage(),
            session_id,
        }
    }
}

#[async_trait::async_trait]
impl Exporter for HtmlExporter {
    async fn export(&self, output_path: &PathBuf) -> Result<ExportStats> {
        let start = std::time::Instant::now();

        // Query all events
        let query = QueryParams {
            session_id: self.session_id.clone(),
            event_type: None,
            from_timestamp: None,
            to_timestamp: None,
            limit: 10000,
            offset: 0,
        };

        let events = self
            .storage
            .backend()
            .query_events(query)
            .await
            .context("Failed to query events")?;

        // Generate HTML report
        let html = format!(
            r#"
<!DOCTYPE html>
<html>
<head>
    <title>SoulBrowser Session Report</title>
    <style>
        body {{ font-family: Arial, sans-serif; margin: 20px; }}
        h1 {{ color: #333; }}
        table {{ border-collapse: collapse; width: 100%; }}
        th, td {{ border: 1px solid #ddd; padding: 8px; text-align: left; }}
        th {{ background-color: #f2f2f2; }}
        tr:nth-child(even) {{ background-color: #f9f9f9; }}
    </style>
</head>
<body>
    <h1>Session Report</h1>
    <p>Total Events: {}</p>
    <table>
        <tr>
            <th>Timestamp</th>
            <th>Event Type</th>
            <th>Session ID</th>
            <th>Details</th>
        </tr>
        {}
    </table>
</body>
</html>"#,
            events.len(),
            events
                .iter()
                .map(|e| format!(
                    "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
                    e.timestamp,
                    e.event_type,
                    e.session_id,
                    serde_json::to_string(&e.data).unwrap_or_else(|_| "{}".to_string())
                ))
                .collect::<Vec<_>>()
                .join("\n")
        );

        // Write HTML file
        let mut file =
            std::fs::File::create(output_path).context("Failed to create output file")?;
        file.write_all(html.as_bytes())
            .context("Failed to write HTML")?;

        Ok(ExportStats {
            total_events: events.len(),
            total_sessions: 1,
            bytes_written: html.len() as u64,
            duration_ms: start.elapsed().as_millis() as u64,
        })
    }
}
