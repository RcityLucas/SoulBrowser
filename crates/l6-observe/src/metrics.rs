use crate::guard::{sanitize_labels, LabelMap};
use crate::policy::current_policy;
use hdrhistogram::Histogram;
use once_cell::sync::OnceCell;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::time::{Duration, Instant};

const WINDOW: Duration = Duration::from_secs(10);

#[derive(Hash, Eq, PartialEq, Clone)]
struct MetricKey {
    name: &'static str,
    labels: Vec<(String, String)>,
}

impl MetricKey {
    fn new(name: &'static str, labels: LabelMap) -> Self {
        let sanitized = sanitize_labels(labels);
        let mut labels_vec: Vec<(String, String)> = sanitized.into_iter().collect();
        labels_vec.sort_by(|a, b| a.0.cmp(&b.0));
        Self {
            name,
            labels: labels_vec,
        }
    }

    fn fmt_labels(&self) -> String {
        if self.labels.is_empty() {
            String::new()
        } else {
            let inner = self
                .labels
                .iter()
                .map(|(k, v)| format!("{}=\"{}\"", k, v))
                .collect::<Vec<_>>()
                .join(",");
            format!("{{{}}}", inner)
        }
    }
}

struct HistEntry {
    histogram: Histogram<u64>,
    sum: u128,
    window_start: Instant,
}

impl HistEntry {
    fn new() -> Self {
        Self {
            histogram: Histogram::<u64>::new(3).expect("create histogram"),
            sum: 0,
            window_start: Instant::now(),
        }
    }
}

static COUNTERS: OnceCell<Mutex<HashMap<MetricKey, u64>>> = OnceCell::new();
static GAUGES: OnceCell<Mutex<HashMap<MetricKey, f64>>> = OnceCell::new();
static HISTOGRAMS: OnceCell<Mutex<HashMap<MetricKey, HistEntry>>> = OnceCell::new();

fn counters() -> &'static Mutex<HashMap<MetricKey, u64>> {
    COUNTERS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn gauges() -> &'static Mutex<HashMap<MetricKey, f64>> {
    GAUGES.get_or_init(|| Mutex::new(HashMap::new()))
}

fn histograms() -> &'static Mutex<HashMap<MetricKey, HistEntry>> {
    HISTOGRAMS.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn ensure_metrics() {
    let _ = counters();
    let _ = gauges();
    let _ = histograms();
}

pub fn inc(name: &'static str, labels: LabelMap) {
    if !current_policy().enable_metrics {
        return;
    }
    let key = MetricKey::new(name, labels);
    let mut map = counters().lock();
    *map.entry(key).or_insert(0) += 1;
}

pub fn set(name: &'static str, value: f64, labels: LabelMap) {
    if !current_policy().enable_metrics {
        return;
    }
    let key = MetricKey::new(name, labels);
    let mut map = gauges().lock();
    map.insert(key, value);
}

pub fn observe(name: &'static str, value: u64, labels: LabelMap) {
    if !current_policy().enable_metrics {
        return;
    }
    let key = MetricKey::new(name, labels);
    let mut map = histograms().lock();
    let entry = map.entry(key).or_insert_with(HistEntry::new);
    let _ = entry.histogram.record(value);
    entry.sum += value as u128;
    if entry.window_start.elapsed() >= WINDOW {
        entry.histogram.reset();
        entry.sum = 0;
        entry.window_start = Instant::now();
    }
}

pub fn render_prometheus() -> String {
    let mut output = String::new();

    // Counters
    for (key, value) in counters().lock().iter() {
        output.push_str(&format!("{}{} {}\n", key.name, key.fmt_labels(), value));
    }

    // Gauges
    for (key, value) in gauges().lock().iter() {
        output.push_str(&format!("{}{} {:.6}\n", key.name, key.fmt_labels(), value));
    }

    // Histograms -> export quantiles and count/sum
    for (key, entry) in histograms().lock().iter() {
        if entry.histogram.len() == 0 {
            continue;
        }
        let quantiles = [0.5, 0.9, 0.95];
        for q in quantiles.iter() {
            let v = entry.histogram.value_at_quantile(*q);
            let mut labels = key
                .labels
                .iter()
                .map(|(k, v)| format!("{}=\"{}\"", k, v))
                .collect::<Vec<_>>();
            labels.push(format!("quantile=\"{:.2}\"", q));
            let label_str = if labels.is_empty() {
                String::new()
            } else {
                format!("{{{}}}", labels.join(","))
            };
            output.push_str(&format!("{}{} {}\n", key.name, label_str, v));
        }
        output.push_str(&format!(
            "{}_count{} {}\n",
            key.name,
            key.fmt_labels(),
            entry.histogram.len()
        ));
        output.push_str(&format!(
            "{}_sum{} {}\n",
            key.name,
            key.fmt_labels(),
            entry.sum
        ));
    }

    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_metrics_api() {
        ensure_metrics();
        let mut labels = LabelMap::new();
        labels.insert("tool".into(), "click".into());
        inc("test_counter", labels.clone());
        set("test_gauge", 42.0, labels.clone());
        observe("test_histogram", 100, labels);
        let rendered = render_prometheus();
        assert!(rendered.contains("test_counter"));
    }
}
