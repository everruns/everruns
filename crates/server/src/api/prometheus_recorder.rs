// In-tree Prometheus recorder and text exposition renderer.
//
// Decision: replaces `metrics-exporter-prometheus`, which pulled `metrics-util`,
// `loom`, `generator`, `evmap`, `left-right`, and `hashbag` to support a rolling
// quantile sketch this deployment does not want. Everything below is exact
// counting plus fixed bucket boundaries.
//
// Decision: histograms render as Prometheus histograms (`_bucket`/`_sum`/
// `_count`), not as summaries. The previous exporter defaulted to summaries,
// whose quantiles cannot be aggregated across replicas — which contradicted the
// horizontal-scaling model documented on the `/metrics` endpoint and in
// knowledge/operations/prometheus-metrics.md. Buckets aggregate correctly with
// `sum without(instance)` and support `histogram_quantile`.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

use metrics::{
    Counter, CounterFn, Gauge, GaugeFn, Histogram, HistogramFn, Key, KeyName, Metadata, Recorder,
    SharedString, Unit,
};

/// Bucket boundaries in seconds, shared by every duration histogram.
///
/// The Prometheus client default (5ms–10s) covers HTTP and tool latency; 30s and
/// 60s are appended because LLM requests routinely run past 10s and would
/// otherwise all land in `+Inf`, leaving no usable quantile above the top bucket.
const BUCKET_BOUNDS_SECONDS: &[f64] = &[
    0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0, 60.0,
];

/// A metric name plus its rendered label set, which together identify one series.
#[derive(Clone, PartialEq, Eq, Hash)]
struct SeriesId {
    name: String,
    labels: Vec<(String, String)>,
}

impl SeriesId {
    fn from_key(key: &Key) -> Self {
        let mut labels: Vec<(String, String)> = key
            .labels()
            .map(|label| (label.key().to_string(), label.value().to_string()))
            .collect();
        // Sort so a series is identified independently of label order at the
        // call site, and so rendered output is stable between scrapes.
        labels.sort();
        Self {
            name: key.name().to_string(),
            labels,
        }
    }
}

/// A counter as an atomic u64, matching Prometheus counter semantics.
#[derive(Default)]
struct CounterState(AtomicU64);

impl CounterFn for CounterState {
    fn increment(&self, value: u64) {
        self.0.fetch_add(value, Ordering::Relaxed);
    }

    fn absolute(&self, value: u64) {
        // `absolute` synchronizes with an external counter that other callers
        // may also be reading, so a stale smaller value must never move the
        // counter backwards.
        self.0.fetch_max(value, Ordering::Relaxed);
    }
}

/// A gauge stored as the bit pattern of an f64 so it can live in an atomic.
#[derive(Default)]
struct GaugeState(AtomicU64);

impl GaugeState {
    fn get(&self) -> f64 {
        f64::from_bits(self.0.load(Ordering::Relaxed))
    }

    /// Applies `op` to the current value, retrying until no other writer
    /// intervened.
    fn update(&self, op: impl Fn(f64) -> f64) {
        let mut current = self.0.load(Ordering::Relaxed);
        loop {
            let next = op(f64::from_bits(current)).to_bits();
            match self.0.compare_exchange_weak(
                current,
                next,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => return,
                Err(actual) => current = actual,
            }
        }
    }
}

impl GaugeFn for GaugeState {
    fn increment(&self, value: f64) {
        self.update(|current| current + value);
    }

    fn decrement(&self, value: f64) {
        self.update(|current| current - value);
    }

    fn set(&self, value: f64) {
        self.0.store(value.to_bits(), Ordering::Relaxed);
    }
}

/// A fixed-bucket histogram. `buckets[i]` counts observations `<=
/// BUCKET_BOUNDS_SECONDS[i]`; `count` is the `+Inf` bucket.
struct HistogramState {
    buckets: Vec<AtomicU64>,
    count: AtomicU64,
    /// Sum of observations, as f64 bits.
    sum: AtomicU64,
}

impl Default for HistogramState {
    fn default() -> Self {
        Self {
            buckets: BUCKET_BOUNDS_SECONDS.iter().map(|_| AtomicU64::new(0)).collect(),
            count: AtomicU64::new(0),
            sum: AtomicU64::new(0),
        }
    }
}

impl HistogramFn for HistogramState {
    fn record(&self, value: f64) {
        // NaN has no bucket and would poison the sum.
        if value.is_nan() {
            return;
        }

        // Bounds are ascending, so the first bucket the value fits in and every
        // bucket after it are cumulative counts.
        for (index, bound) in BUCKET_BOUNDS_SECONDS.iter().enumerate() {
            if value <= *bound {
                self.buckets[index].fetch_add(1, Ordering::Relaxed);
            }
        }
        self.count.fetch_add(1, Ordering::Relaxed);

        let mut current = self.sum.load(Ordering::Relaxed);
        loop {
            let next = (f64::from_bits(current) + value).to_bits();
            match self.sum.compare_exchange_weak(
                current,
                next,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(actual) => current = actual,
            }
        }
    }
}

/// Metric metadata registered through the `describe_*` macros.
#[derive(Default, Clone)]
struct Description {
    unit: Option<Unit>,
    text: String,
}

#[derive(Default)]
struct Registry {
    counters: RwLock<HashMap<SeriesId, Arc<CounterState>>>,
    gauges: RwLock<HashMap<SeriesId, Arc<GaugeState>>>,
    histograms: RwLock<HashMap<SeriesId, Arc<HistogramState>>>,
    descriptions: RwLock<HashMap<String, Description>>,
}

impl Registry {
    /// Returns the handle for `key`, creating it on first use.
    fn entry<T: Default>(store: &RwLock<HashMap<SeriesId, Arc<T>>>, key: &Key) -> Arc<T> {
        let id = SeriesId::from_key(key);
        if let Some(existing) = store.read().expect("metrics registry poisoned").get(&id) {
            return Arc::clone(existing);
        }
        Arc::clone(
            store
                .write()
                .expect("metrics registry poisoned")
                .entry(id)
                .or_default(),
        )
    }

    fn describe(&self, key: KeyName, unit: Option<Unit>, description: SharedString) {
        self.descriptions
            .write()
            .expect("metrics registry poisoned")
            .insert(
                key.as_str().to_string(),
                Description {
                    unit,
                    text: description.to_string(),
                },
            );
    }
}

/// A `metrics::Recorder` that renders the Prometheus text exposition format.
pub struct PrometheusRecorder {
    registry: Arc<Registry>,
}

impl PrometheusRecorder {
    fn new() -> Self {
        Self {
            registry: Arc::new(Registry::default()),
        }
    }

    /// A handle that renders the current values. Cheap to clone.
    pub fn handle(&self) -> PrometheusHandle {
        PrometheusHandle {
            registry: Arc::clone(&self.registry),
        }
    }
}

impl Recorder for PrometheusRecorder {
    fn describe_counter(&self, key: KeyName, unit: Option<Unit>, description: SharedString) {
        self.registry.describe(key, unit, description);
    }

    fn describe_gauge(&self, key: KeyName, unit: Option<Unit>, description: SharedString) {
        self.registry.describe(key, unit, description);
    }

    fn describe_histogram(&self, key: KeyName, unit: Option<Unit>, description: SharedString) {
        self.registry.describe(key, unit, description);
    }

    fn register_counter(&self, key: &Key, _metadata: &Metadata<'_>) -> Counter {
        Counter::from_arc(Registry::entry(&self.registry.counters, key))
    }

    fn register_gauge(&self, key: &Key, _metadata: &Metadata<'_>) -> Gauge {
        Gauge::from_arc(Registry::entry(&self.registry.gauges, key))
    }

    fn register_histogram(&self, key: &Key, _metadata: &Metadata<'_>) -> Histogram {
        Histogram::from_arc(Registry::entry(&self.registry.histograms, key))
    }
}

/// Renders the registered metrics. Obtained from [`install_recorder`].
#[derive(Clone)]
pub struct PrometheusHandle {
    registry: Arc<Registry>,
}

impl PrometheusHandle {
    /// Renders every metric in the Prometheus text exposition format.
    pub fn render(&self) -> String {
        let descriptions = self
            .registry
            .descriptions
            .read()
            .expect("metrics registry poisoned")
            .clone();
        let mut out = String::new();

        let counters = self
            .registry
            .counters
            .read()
            .expect("metrics registry poisoned");
        for (name, series) in group_by_name(counters.iter().map(|(id, state)| {
            (id, state.0.load(Ordering::Relaxed) as f64)
        })) {
            write_metric_header(&mut out, &name, "counter", descriptions.get(&name));
            for (id, value) in series {
                write_sample(&mut out, &name, &id.labels, None, value);
            }
        }
        drop(counters);

        let gauges = self
            .registry
            .gauges
            .read()
            .expect("metrics registry poisoned");
        for (name, series) in group_by_name(gauges.iter().map(|(id, state)| (id, state.get()))) {
            write_metric_header(&mut out, &name, "gauge", descriptions.get(&name));
            for (id, value) in series {
                write_sample(&mut out, &name, &id.labels, None, value);
            }
        }
        drop(gauges);

        let histograms = self
            .registry
            .histograms
            .read()
            .expect("metrics registry poisoned");
        let mut by_name: HashMap<String, Vec<(&SeriesId, &Arc<HistogramState>)>> = HashMap::new();
        for (id, state) in histograms.iter() {
            by_name.entry(id.name.clone()).or_default().push((id, state));
        }
        let mut names: Vec<&String> = by_name.keys().collect();
        names.sort();
        for name in names {
            write_metric_header(&mut out, name, "histogram", descriptions.get(name));
            let mut series = by_name[name].clone();
            series.sort_by(|(a, _), (b, _)| a.labels.cmp(&b.labels));
            for (id, state) in series {
                for (index, bound) in BUCKET_BOUNDS_SECONDS.iter().enumerate() {
                    let count = state.buckets[index].load(Ordering::Relaxed);
                    write_sample(
                        &mut out,
                        &format!("{name}_bucket"),
                        &id.labels,
                        Some(("le", &format_float(*bound))),
                        count as f64,
                    );
                }
                let count = state.count.load(Ordering::Relaxed);
                write_sample(
                    &mut out,
                    &format!("{name}_bucket"),
                    &id.labels,
                    Some(("le", "+Inf")),
                    count as f64,
                );
                write_sample(
                    &mut out,
                    &format!("{name}_sum"),
                    &id.labels,
                    None,
                    f64::from_bits(state.sum.load(Ordering::Relaxed)),
                );
                write_sample(
                    &mut out,
                    &format!("{name}_count"),
                    &id.labels,
                    None,
                    count as f64,
                );
            }
        }

        out
    }
}

/// Groups series by metric name, with both names and label sets sorted so
/// consecutive scrapes produce identical output.
fn group_by_name<'a>(
    entries: impl Iterator<Item = (&'a SeriesId, f64)>,
) -> Vec<(String, Vec<(&'a SeriesId, f64)>)> {
    let mut grouped: HashMap<String, Vec<(&SeriesId, f64)>> = HashMap::new();
    for (id, value) in entries {
        grouped.entry(id.name.clone()).or_default().push((id, value));
    }
    let mut out: Vec<(String, Vec<(&SeriesId, f64)>)> = grouped.into_iter().collect();
    out.sort_by(|(a, _), (b, _)| a.cmp(b));
    for (_, series) in &mut out {
        series.sort_by(|(a, _), (b, _)| a.labels.cmp(&b.labels));
    }
    out
}

fn write_metric_header(out: &mut String, name: &str, kind: &str, description: Option<&Description>) {
    if let Some(description) = description {
        let mut help = description.text.clone();
        if let Some(unit) = description.unit {
            // Keep the unit visible; Prometheus has no separate unit field in
            // the 0.0.4 exposition format.
            help = format!("{help} (unit: {})", unit.as_str());
        }
        if !help.is_empty() {
            let _ = writeln!(out, "# HELP {name} {}", escape_help(&help));
        }
    }
    let _ = writeln!(out, "# TYPE {name} {kind}");
}

fn write_sample(
    out: &mut String,
    name: &str,
    labels: &[(String, String)],
    extra: Option<(&str, &str)>,
    value: f64,
) {
    let _ = write!(out, "{name}");
    if !labels.is_empty() || extra.is_some() {
        let mut rendered: Vec<String> = labels
            .iter()
            .map(|(key, value)| format!("{key}=\"{}\"", escape_label_value(value)))
            .collect();
        if let Some((key, value)) = extra {
            rendered.push(format!("{key}=\"{}\"", escape_label_value(value)));
        }
        let _ = write!(out, "{{{}}}", rendered.join(","));
    }
    let _ = writeln!(out, " {}", format_float(value));
}

/// Formats a float the way Prometheus expects: integers without a decimal
/// point, infinities as `+Inf`/`-Inf`.
fn format_float(value: f64) -> String {
    if value.is_infinite() {
        return if value.is_sign_positive() { "+Inf".into() } else { "-Inf".into() };
    }
    if value.is_nan() {
        return "NaN".into();
    }
    if value == value.trunc() && value.abs() < 1e15 {
        return format!("{}", value as i64);
    }
    format!("{value}")
}

/// `\` and newlines are escaped in HELP text.
fn escape_help(text: &str) -> String {
    text.replace('\\', "\\\\").replace('\n', "\\n")
}

/// `\`, `"` and newlines are escaped in label values.
fn escape_label_value(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}

/// Installs the recorder globally and returns a render handle.
///
/// Fails if a global recorder is already installed, which can only happen once
/// per process.
pub fn install_recorder() -> Result<PrometheusHandle, metrics::SetRecorderError<PrometheusRecorder>>
{
    let recorder = PrometheusRecorder::new();
    let handle = recorder.handle();
    metrics::set_global_recorder(recorder)?;
    Ok(handle)
}

#[cfg(test)]
mod tests {
    use super::*;
    use metrics::{Label, Metadata};

    fn key(name: &'static str, labels: &[(&'static str, &'static str)]) -> Key {
        Key::from_parts(
            name,
            labels
                .iter()
                .map(|(k, v)| Label::new(*k, *v))
                .collect::<Vec<_>>(),
        )
    }

    const META: Metadata<'static> = Metadata::new("test", metrics::Level::INFO, None);

    #[test]
    fn counters_render_with_sorted_labels() {
        let recorder = PrometheusRecorder::new();
        let handle = recorder.handle();

        recorder
            .register_counter(&key("requests_total", &[("z", "1"), ("a", "2")]), &META)
            .increment(3);

        let rendered = handle.render();
        assert!(rendered.contains("# TYPE requests_total counter"));
        assert!(rendered.contains("requests_total{a=\"2\",z=\"1\"} 3"));
    }

    #[test]
    fn counter_absolute_never_moves_backwards() {
        let recorder = PrometheusRecorder::new();
        let handle = recorder.handle();
        let counter = recorder.register_counter(&key("c", &[]), &META);

        counter.absolute(10);
        counter.absolute(4);

        assert!(handle.render().contains("c 10"));
    }

    #[test]
    fn gauges_support_set_increment_and_decrement() {
        let recorder = PrometheusRecorder::new();
        let handle = recorder.handle();
        let gauge = recorder.register_gauge(&key("g", &[]), &META);

        gauge.set(10.0);
        gauge.increment(5.0);
        gauge.decrement(2.5);

        assert!(handle.render().contains("g 12.5"), "{}", handle.render());
    }

    #[test]
    fn histograms_render_cumulative_buckets_sum_and_count() {
        let recorder = PrometheusRecorder::new();
        let handle = recorder.handle();
        let histogram = recorder.register_histogram(&key("d_seconds", &[]), &META);

        histogram.record(0.02);
        histogram.record(3.0);

        let rendered = handle.render();
        assert!(rendered.contains("# TYPE d_seconds histogram"));
        // 0.02 lands in the 0.025 bucket and every bucket above it.
        assert!(rendered.contains("d_seconds_bucket{le=\"0.01\"} 0"));
        assert!(rendered.contains("d_seconds_bucket{le=\"0.025\"} 1"));
        assert!(rendered.contains("d_seconds_bucket{le=\"2.5\"} 1"));
        assert!(rendered.contains("d_seconds_bucket{le=\"5\"} 2"));
        assert!(rendered.contains("d_seconds_bucket{le=\"+Inf\"} 2"));
        assert!(rendered.contains("d_seconds_sum 3.02"));
        assert!(rendered.contains("d_seconds_count 2"));
    }

    #[test]
    fn histogram_observations_above_the_top_bucket_still_count() {
        let recorder = PrometheusRecorder::new();
        let handle = recorder.handle();
        recorder
            .register_histogram(&key("d_seconds", &[]), &META)
            .record(600.0);

        let rendered = handle.render();
        assert!(rendered.contains("d_seconds_bucket{le=\"60\"} 0"));
        assert!(rendered.contains("d_seconds_bucket{le=\"+Inf\"} 1"));
        assert!(rendered.contains("d_seconds_sum 600"));
    }

    #[test]
    fn histogram_ignores_nan() {
        let recorder = PrometheusRecorder::new();
        let handle = recorder.handle();
        recorder
            .register_histogram(&key("d_seconds", &[]), &META)
            .record(f64::NAN);

        assert!(handle.render().contains("d_seconds_count 0"));
    }

    #[test]
    fn label_values_are_escaped() {
        let recorder = PrometheusRecorder::new();
        let handle = recorder.handle();
        recorder
            .register_counter(&key("c", &[("path", "a\"b\\c\nd")]), &META)
            .increment(1);

        assert!(
            handle
                .render()
                .contains("c{path=\"a\\\"b\\\\c\\nd\"} 1"),
            "{}",
            handle.render()
        );
    }

    #[test]
    fn describe_emits_help_text() {
        let recorder = PrometheusRecorder::new();
        let handle = recorder.handle();
        recorder.describe_counter(
            KeyName::from("c"),
            Some(Unit::Seconds),
            SharedString::from("how many"),
        );
        recorder.register_counter(&key("c", &[]), &META).increment(1);

        let rendered = handle.render();
        assert!(rendered.contains("# HELP c how many (unit: seconds)"), "{rendered}");
    }

    #[test]
    fn same_key_returns_the_same_series() {
        let recorder = PrometheusRecorder::new();
        let handle = recorder.handle();

        recorder.register_counter(&key("c", &[("a", "1")]), &META).increment(1);
        recorder.register_counter(&key("c", &[("a", "1")]), &META).increment(1);

        let rendered = handle.render();
        assert!(rendered.contains("c{a=\"1\"} 2"));
        assert_eq!(rendered.matches("c{a=\"1\"}").count(), 1);
    }

    #[test]
    fn render_is_stable_across_scrapes() {
        let recorder = PrometheusRecorder::new();
        let handle = recorder.handle();
        for value in ["b", "a", "c"] {
            recorder
                .register_counter(&key("c", &[("v", value)]), &META)
                .increment(1);
        }
        recorder.register_gauge(&key("g", &[]), &META).set(1.0);

        assert_eq!(handle.render(), handle.render());
    }
}
