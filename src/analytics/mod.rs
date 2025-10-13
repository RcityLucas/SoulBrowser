//! Analytics and analysis module
//!
//! Provides analytics and insights from recorded session data

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

use crate::{
    app_context::AppContext,
    storage::{BrowserEvent, QueryParams, StorageManager},
};

/// Session analyzer for generating insights
pub struct SessionAnalyzer {
    storage: Arc<StorageManager>,
}

impl SessionAnalyzer {
    /// Create analyzer with app context
    pub fn with_context(context: Arc<AppContext>) -> Self {
        Self {
            storage: context.storage(),
        }
    }

    /// Analyze a specific session
    pub async fn analyze_session(&self, session_id: &str) -> Result<SessionAnalytics> {
        let events = self.load_events(session_id).await?;

        if events.is_empty() {
            return Ok(SessionAnalytics::default());
        }

        // Calculate analytics
        let mut event_counts = HashMap::new();
        let mut page_visits = HashMap::new();

        let first_timestamp = events.first().map(|e| e.timestamp).unwrap_or(0);
        let last_timestamp = events.last().map(|e| e.timestamp).unwrap_or(0);
        let total_duration = last_timestamp - first_timestamp;

        for event in &events {
            // Count event types
            *event_counts.entry(event.event_type.clone()).or_insert(0) += 1;

            // Track page visits
            if event.event_type == "navigate" {
                if let Some(url) = event.data.get("url").and_then(|v| v.as_str()) {
                    *page_visits.entry(url.to_string()).or_insert(0) += 1;
                }
            }
        }

        Ok(SessionAnalytics {
            session_id: session_id.to_string(),
            total_events: events.len(),
            duration_ms: total_duration,
            event_types: event_counts,
            page_visits,
            start_time: first_timestamp,
            end_time: last_timestamp,
        })
    }

    /// Generate aggregated analytics across all sessions
    pub async fn generate_report(&self) -> Result<AnalyticsReport> {
        // Get all sessions
        let sessions = self
            .storage
            .backend()
            .list_sessions()
            .await
            .context("Failed to list sessions")?;

        let mut total_events = 0;
        let mut total_duration = 0i64;
        let mut event_type_totals = HashMap::new();
        let mut page_visit_totals = HashMap::new();

        for session in &sessions {
            let analytics = self.analyze_session(&session.id).await?;

            total_events += analytics.total_events;
            total_duration += analytics.duration_ms;

            for (event_type, count) in analytics.event_types {
                *event_type_totals.entry(event_type).or_insert(0) += count;
            }

            for (page, visits) in analytics.page_visits {
                *page_visit_totals.entry(page).or_insert(0) += visits;
            }
        }

        // Find most common events and pages
        let most_common_event = event_type_totals
            .iter()
            .max_by_key(|(_, count)| *count)
            .map(|(k, v)| (k.clone(), *v));

        let most_visited_page = page_visit_totals
            .iter()
            .max_by_key(|(_, count)| *count)
            .map(|(k, v)| (k.clone(), *v));

        Ok(AnalyticsReport {
            total_sessions: sessions.len(),
            total_events,
            total_duration_ms: total_duration,
            average_events_per_session: if sessions.is_empty() {
                0.0
            } else {
                total_events as f64 / sessions.len() as f64
            },
            average_duration_ms: if sessions.is_empty() {
                0
            } else {
                total_duration / sessions.len() as i64
            },
            event_type_distribution: event_type_totals,
            page_visit_distribution: page_visit_totals,
            most_common_event,
            most_visited_page,
        })
    }

    /// Retrieve raw events for a session sorted by timestamp
    pub async fn session_events(&self, session_id: &str) -> Result<Vec<BrowserEvent>> {
        self.load_events(session_id).await
    }

    async fn load_events(&self, session_id: &str) -> Result<Vec<BrowserEvent>> {
        let query = QueryParams {
            session_id: Some(session_id.to_string()),
            event_type: None,
            from_timestamp: None,
            to_timestamp: None,
            limit: 0,
            offset: 0,
        };

        let mut events = self
            .storage
            .backend()
            .query_events(query)
            .await
            .context("Failed to query events")?;

        events.sort_by(|a, b| {
            a.timestamp
                .cmp(&b.timestamp)
                .then_with(|| a.sequence.cmp(&b.sequence))
        });

        Ok(events)
    }
}

/// Analytics for a single session
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct SessionAnalytics {
    pub session_id: String,
    pub total_events: usize,
    pub duration_ms: i64,
    pub event_types: HashMap<String, usize>,
    pub page_visits: HashMap<String, usize>,
    pub start_time: i64,
    pub end_time: i64,
}

/// Aggregated analytics report
#[derive(Debug, Serialize, Deserialize)]
pub struct AnalyticsReport {
    pub total_sessions: usize,
    pub total_events: usize,
    pub total_duration_ms: i64,
    pub average_events_per_session: f64,
    pub average_duration_ms: i64,
    pub event_type_distribution: HashMap<String, usize>,
    pub page_visit_distribution: HashMap<String, usize>,
    pub most_common_event: Option<(String, usize)>,
    pub most_visited_page: Option<(String, usize)>,
}
