use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use anyhow::{bail, Result};
use clap::Args;
use serde::Serialize;
use serde_json::{self, json};
use tokio::fs;
use tracing::info;

use crate::cli::context::CliContext;
use soulbrowser_kernel::analytics::{SessionAnalytics, SessionAnalyzer};
use soulbrowser_kernel::storage::BrowserEvent;

#[derive(Args, Clone, Debug)]
pub struct AnalyzeArgs {
    /// Session or recording to analyze
    pub target: String,

    /// Analysis type
    #[arg(short, long, default_value = "performance")]
    pub analysis_type: AnalysisType,

    /// Output report file
    #[arg(short, long)]
    pub output: Option<PathBuf>,

    /// Generate interactive report
    #[arg(long)]
    pub interactive: bool,
}

#[derive(Clone, Debug, clap::ValueEnum)]
pub enum AnalysisType {
    Performance,
    Accessibility,
    Security,
    Usability,
    Compatibility,
    Full,
}

pub async fn cmd_analyze(args: AnalyzeArgs, ctx: &CliContext) -> Result<()> {
    info!("Analyzing session: {}", args.target);

    let context = ctx.app_context().await?;

    let analyzer = SessionAnalyzer::with_context(context.clone());

    if args.target.eq_ignore_ascii_case("all") {
        let report = analyzer.generate_report().await?;

        println!("Portfolio Analysis (all sessions):");
        println!("- Total sessions: {}", report.total_sessions);
        println!("- Total events: {}", report.total_events);
        println!(
            "- Average events per session: {:.2}",
            report.average_events_per_session
        );
        println!(
            "- Average session duration: {} ms",
            report.average_duration_ms
        );

        if let Some((ref event, count)) = report.most_common_event {
            println!("- Most common event: {} ({} times)", event, count);
        }

        if let Some((ref page, visits)) = report.most_visited_page {
            println!("- Most visited page: {} ({} visits)", page, visits);
        }

        if let Some(output_path) = args.output {
            let json = serde_json::to_string_pretty(&report)?;
            fs::write(output_path.clone(), json).await?;
            info!("Aggregated report saved to: {}", output_path.display());
        }
        return Ok(());
    }

    let events = analyzer.session_events(&args.target).await?;

    if events.is_empty() {
        bail!(
            "No events found for session {}. Try recording or running automation first.",
            args.target
        );
    }

    let analytics = analyzer.analyze_session(&args.target).await?;

    let performance = build_performance_report(&args.target, &analytics, &events);
    let accessibility = build_accessibility_report(&args.target, &events);
    let security = build_security_report(&args.target, &events);
    let usability = build_usability_report(&args.target, &events);
    let compatibility = build_compatibility_report(&args.target, &events);

    if args.interactive {
        println!("Interactive reporting is not available in CLI mode; rendering static insights.");
        println!();
    }

    match args.analysis_type {
        AnalysisType::Full => {
            println!("Session analysis summary for {}:", args.target);
            println!("- Total events: {}", analytics.total_events);
            println!("- Duration: {} ms", analytics.duration_ms);
            println!("- Unique event types: {}", analytics.event_types.len());
            println!("- Pages visited: {}", analytics.page_visits.len());
            println!();

            print_performance_summary(&performance);
            println!();
            print_accessibility_summary(&accessibility);
            println!();
            print_security_summary(&security);
            println!();
            print_usability_summary(&usability);
            println!();
            print_compatibility_summary(&compatibility);

            if let Some(output_path) = args.output {
                let bundle = json!({
                    "session": analytics,
                    "performance": performance,
                    "accessibility": accessibility,
                    "security": security,
                    "usability": usability,
                    "compatibility": compatibility,
                });
                let json = serde_json::to_string_pretty(&bundle)?;
                fs::write(output_path.clone(), json).await?;
                info!("Detailed analysis saved to: {}", output_path.display());
            }
        }
        AnalysisType::Performance => {
            println!("Performance analysis for {}:", args.target);
            print_performance_summary(&performance);
            if let Some(output_path) = args.output {
                let json = serde_json::to_string_pretty(&performance)?;
                fs::write(output_path.clone(), json).await?;
                info!("Performance report saved to: {}", output_path.display());
            }
        }
        AnalysisType::Accessibility => {
            println!("Accessibility analysis for {}:", args.target);
            print_accessibility_summary(&accessibility);
            if let Some(output_path) = args.output {
                let json = serde_json::to_string_pretty(&accessibility)?;
                fs::write(output_path.clone(), json).await?;
                info!("Accessibility report saved to: {}", output_path.display());
            }
        }
        AnalysisType::Security => {
            println!("Security analysis for {}:", args.target);
            print_security_summary(&security);
            if let Some(output_path) = args.output {
                let json = serde_json::to_string_pretty(&security)?;
                fs::write(output_path.clone(), json).await?;
                info!("Security report saved to: {}", output_path.display());
            }
        }
        AnalysisType::Usability => {
            println!("Usability analysis for {}:", args.target);
            print_usability_summary(&usability);
            if let Some(output_path) = args.output {
                let json = serde_json::to_string_pretty(&usability)?;
                fs::write(output_path.clone(), json).await?;
                info!("Usability report saved to: {}", output_path.display());
            }
        }
        AnalysisType::Compatibility => {
            println!("Compatibility analysis for {}:", args.target);
            print_compatibility_summary(&compatibility);
            if let Some(output_path) = args.output {
                let json = serde_json::to_string_pretty(&compatibility)?;
                fs::write(output_path.clone(), json).await?;
                info!("Compatibility report saved to: {}", output_path.display());
            }
        }
    }

    Ok(())
}

#[derive(Debug, Serialize)]
struct IdlePeriodReport {
    from_event: String,
    to_event: String,
    duration_ms: i64,
}

#[derive(Debug, Serialize)]
struct PerformanceReport {
    session_id: String,
    total_events: usize,
    duration_ms: i64,
    events_per_minute: f64,
    average_gap_ms: Option<f64>,
    longest_gap_ms: Option<i64>,
    idle_periods: Vec<IdlePeriodReport>,
}

#[derive(Debug, Serialize)]
struct AccessibilityReport {
    session_id: String,
    total_interactions: usize,
    accessible_interactions: usize,
    accessibility_score: f64,
    selectors_missing_accessibility: Vec<String>,
}

#[derive(Debug, Serialize)]
struct SecurityReport {
    session_id: String,
    total_navigations: usize,
    insecure_urls: Vec<String>,
    sensitive_selectors: Vec<String>,
    warnings: Vec<String>,
}

#[derive(Debug, Serialize)]
struct SelectorCount {
    selector: String,
    count: usize,
}

#[derive(Debug, Serialize)]
struct UsabilityReport {
    session_id: String,
    repeated_clicks: Vec<SelectorCount>,
    repeated_inputs: Vec<SelectorCount>,
    slow_segments: Vec<IdlePeriodReport>,
}

#[derive(Debug, Serialize)]
struct CompatibilityReport {
    session_id: String,
    navigation_count: usize,
    unique_domains: Vec<String>,
    schemes_used: Vec<String>,
    mixed_content: bool,
}

fn build_performance_report(
    session_id: &str,
    analytics: &SessionAnalytics,
    events: &[BrowserEvent],
) -> PerformanceReport {
    let total_events = events.len();
    let duration_ms = analytics.duration_ms;

    let mut gaps = Vec::new();
    let mut idle_periods = Vec::new();

    for window in events.windows(2) {
        if let [prev, next] = window {
            let gap = next.timestamp.saturating_sub(prev.timestamp);
            gaps.push(gap);
            if gap as i64 >= 1_500 {
                idle_periods.push(IdlePeriodReport {
                    from_event: describe_event(prev),
                    to_event: describe_event(next),
                    duration_ms: gap as i64,
                });
            }
        }
    }

    let average_gap = if !gaps.is_empty() {
        Some(gaps.iter().sum::<i64>() as f64 / gaps.len() as f64)
    } else {
        None
    };

    let longest_gap = gaps.into_iter().max();

    let events_per_minute = if duration_ms > 0 {
        let minutes = duration_ms as f64 / 60_000.0;
        total_events as f64 / minutes
    } else {
        total_events as f64
    };

    PerformanceReport {
        session_id: session_id.to_string(),
        total_events,
        duration_ms,
        events_per_minute,
        average_gap_ms: average_gap,
        longest_gap_ms: longest_gap,
        idle_periods,
    }
}

fn build_accessibility_report(session_id: &str, events: &[BrowserEvent]) -> AccessibilityReport {
    let mut accessible = 0usize;
    let mut total = 0usize;
    let mut missing = HashSet::new();

    for event in events {
        if let Some(selector) = event.data.get("selector").and_then(|v| v.as_str()) {
            total += 1;
            if is_accessible_selector(selector) {
                accessible += 1;
            } else {
                missing.insert(selector.to_string());
            }
        }
    }

    AccessibilityReport {
        session_id: session_id.to_string(),
        total_interactions: total,
        accessible_interactions: accessible,
        accessibility_score: if total > 0 {
            accessible as f64 / total as f64
        } else {
            1.0
        },
        selectors_missing_accessibility: to_sorted_vec(missing),
    }
}

fn build_security_report(session_id: &str, events: &[BrowserEvent]) -> SecurityReport {
    let mut insecure_urls = Vec::new();
    let mut sensitive_selectors = Vec::new();
    let mut warnings = Vec::new();
    let mut navigation_count = 0usize;

    for event in events {
        match event.event_type.as_str() {
            "navigate" => {
                navigation_count += 1;
                if let Some(url) = event.data.get("url").and_then(|v| v.as_str()) {
                    if url.starts_with("http://") {
                        insecure_urls.push(url.to_string());
                    }
                }
            }
            "type" => {
                if let Some(selector) = event.data.get("selector").and_then(|v| v.as_str()) {
                    let lower = selector.to_lowercase();
                    if lower.contains("password") || lower.contains("credit") {
                        sensitive_selectors.push(selector.to_string());
                    }
                }
            }
            "storage_write" => {
                warnings.push("Storage writes detected; review for secrets".to_string());
            }
            _ => {}
        }
    }

    SecurityReport {
        session_id: session_id.to_string(),
        total_navigations: navigation_count,
        insecure_urls,
        sensitive_selectors,
        warnings,
    }
}

fn build_usability_report(session_id: &str, events: &[BrowserEvent]) -> UsabilityReport {
    let mut click_counts: HashMap<String, usize> = HashMap::new();
    let mut input_counts: HashMap<String, usize> = HashMap::new();
    let mut slow_segments = Vec::new();

    for event in events {
        match event.event_type.as_str() {
            "click" => {
                if let Some(selector) = event.data.get("selector").and_then(|v| v.as_str()) {
                    *click_counts.entry(selector.to_string()).or_default() += 1;
                }
            }
            "type" => {
                if let Some(selector) = event.data.get("selector").and_then(|v| v.as_str()) {
                    *input_counts.entry(selector.to_string()).or_default() += 1;
                }
            }
            _ => {}
        }
    }

    for window in events.windows(2) {
        if let [prev, next] = window {
            let gap = next.timestamp.saturating_sub(prev.timestamp);
            if gap as i64 >= 2_500 {
                slow_segments.push(IdlePeriodReport {
                    from_event: describe_event(prev),
                    to_event: describe_event(next),
                    duration_ms: gap as i64,
                });
            }
        }
    }

    let repeated_clicks = click_counts
        .into_iter()
        .filter(|(_, count)| *count > 1)
        .map(|(selector, count)| SelectorCount { selector, count })
        .collect();

    let repeated_inputs = input_counts
        .into_iter()
        .filter(|(_, count)| *count > 1)
        .map(|(selector, count)| SelectorCount { selector, count })
        .collect();

    UsabilityReport {
        session_id: session_id.to_string(),
        repeated_clicks,
        repeated_inputs,
        slow_segments,
    }
}

fn build_compatibility_report(session_id: &str, events: &[BrowserEvent]) -> CompatibilityReport {
    let mut navigation_count = 0usize;
    let mut domains = HashSet::new();
    let mut schemes = HashSet::new();
    let mut mixed_content = false;

    for event in events {
        if let "navigate" = event.event_type.as_str() {
            navigation_count += 1;
            if let Some(url) = event.data.get("url").and_then(|v| v.as_str()) {
                if let Some((scheme, host)) = parse_url_parts(url) {
                    schemes.insert(scheme);
                    domains.insert(host);
                }
            }
        }
    }

    if schemes.contains("http") && schemes.contains("https") {
        mixed_content = true;
    }

    CompatibilityReport {
        session_id: session_id.to_string(),
        navigation_count,
        unique_domains: to_sorted_vec(domains),
        schemes_used: to_sorted_vec(schemes),
        mixed_content,
    }
}

fn print_performance_summary(report: &PerformanceReport) {
    println!("Performance insights:");
    println!("- Events per minute: {:.2}", report.events_per_minute);
    if let Some(avg) = report.average_gap_ms {
        println!("- Average gap between events: {:.0} ms", avg);
    }
    if let Some(longest) = report.longest_gap_ms {
        println!("- Longest idle period: {} ms", longest);
    }
    if report.idle_periods.is_empty() {
        println!("- No idle periods above 1500 ms detected.");
    } else {
        println!("- Idle periods above 1500 ms:");
        for idle in &report.idle_periods {
            println!(
                "  - {} -> {} ({} ms)",
                idle.from_event, idle.to_event, idle.duration_ms
            );
        }
    }
}

fn print_accessibility_summary(report: &AccessibilityReport) {
    println!("Accessibility insights:");
    println!(
        "- Accessible interactions: {} of {}",
        report.accessible_interactions, report.total_interactions
    );
    println!(
        "- Accessibility score: {:.0}%",
        report.accessibility_score * 100.0
    );
    if report.selectors_missing_accessibility.is_empty() {
        println!("- All selectors include basic accessibility hints.");
    } else {
        println!("- Selectors missing accessibility cues:");
        for selector in &report.selectors_missing_accessibility {
            println!("  - {}", selector);
        }
    }
}

fn print_security_summary(report: &SecurityReport) {
    println!("Security observations:");
    println!("- Navigations observed: {}", report.total_navigations);
    if report.insecure_urls.is_empty() {
        println!("- No HTTP navigations detected.");
    } else {
        println!("- Navigations using HTTP:");
        for url in &report.insecure_urls {
            println!("  - {}", url);
        }
    }
    if report.sensitive_selectors.is_empty() {
        println!("- No sensitive input selectors matched.");
    } else {
        println!("- Sensitive selectors interacted with:");
        for selector in &report.sensitive_selectors {
            println!("  - {}", selector);
        }
    }
    if report.warnings.is_empty() {
        println!("- No security warnings raised.");
    } else {
        for warning in &report.warnings {
            println!("- Warning: {}", warning);
        }
    }
}

fn print_usability_summary(report: &UsabilityReport) {
    println!("Usability highlights:");
    if report.repeated_clicks.is_empty() {
        println!("- No repeated clicks detected on the same selector.");
    } else {
        println!("- Repeated clicks detected:");
        for entry in &report.repeated_clicks {
            println!("  - {} ({} times)", entry.selector, entry.count);
        }
    }
    if report.repeated_inputs.is_empty() {
        println!("- No fields were typed into more than once.");
    } else {
        println!("- Fields edited multiple times:");
        for entry in &report.repeated_inputs {
            println!("  - {} ({} times)", entry.selector, entry.count);
        }
    }
    if report.slow_segments.is_empty() {
        println!("- No slow segments above 2500 ms detected.");
    } else {
        println!("- Slow segments above 2500 ms:");
        for idle in &report.slow_segments {
            println!(
                "  - {} -> {} ({} ms)",
                idle.from_event, idle.to_event, idle.duration_ms
            );
        }
    }
}

fn print_compatibility_summary(report: &CompatibilityReport) {
    println!("Compatibility snapshot:");
    println!("- Navigations performed: {}", report.navigation_count);
    if report.schemes_used.is_empty() {
        println!("- Schemes used: (none)");
    } else {
        println!("- Schemes used: {}", report.schemes_used.join(", "));
    }
    if report.mixed_content {
        println!("- Warning: both HTTP and HTTPS were used; check for mixed content issues.");
    }
    if report.unique_domains.is_empty() {
        println!("- No external domains were visited.");
    } else {
        println!("- Domains visited:");
        for domain in &report.unique_domains {
            println!("  - {}", domain);
        }
    }
}

fn describe_event(event: &BrowserEvent) -> String {
    match event.event_type.as_str() {
        "navigate" => event
            .data
            .get("url")
            .and_then(|v| v.as_str())
            .map(|url| format!("navigate -> {}", url))
            .unwrap_or_else(|| "navigate".to_string()),
        "click" => event
            .data
            .get("selector")
            .and_then(|v| v.as_str())
            .map(|selector| format!("click -> {}", selector))
            .unwrap_or_else(|| "click".to_string()),
        "type" => event
            .data
            .get("selector")
            .and_then(|v| v.as_str())
            .map(|selector| format!("type -> {}", selector))
            .unwrap_or_else(|| "type".to_string()),
        other => other.to_string(),
    }
}

fn is_accessible_selector(selector: &str) -> bool {
    let lower = selector.to_lowercase();
    lower.contains("aria-")
        || lower.contains("[role=")
        || lower.contains("button")
        || lower.contains("label")
        || lower.contains("input")
}

fn parse_url_parts(url: &str) -> Option<(String, String)> {
    let mut parts = url.splitn(2, "://");
    let scheme = parts.next()?;
    let rest = parts.next().unwrap_or("");
    let host = rest.split('/').next().unwrap_or("");
    Some((scheme.to_string(), host.to_string()))
}

fn to_sorted_vec(set: HashSet<String>) -> Vec<String> {
    let mut items: Vec<String> = set.into_iter().collect();
    items.sort();
    items
}
