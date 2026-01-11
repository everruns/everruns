//! HTML report generation (Gatling-style)
//!
//! Generates interactive HTML reports with charts for benchmark results.

use std::fs;
use std::path::Path;

use minijinja::{context, Environment};

use super::metrics::{BenchmarkMetrics, LatencySummary, ResourceSnapshot};

/// Configuration for report generation
#[derive(Debug, Clone)]
pub struct ReportConfig {
    /// Output directory for reports
    pub output_dir: String,
    /// Report title
    pub title: String,
    /// Include raw data in report
    pub include_raw_data: bool,
}

impl Default for ReportConfig {
    fn default() -> Self {
        Self {
            output_dir: "target/benchmark-reports".to_string(),
            title: "Benchmark Report".to_string(),
            include_raw_data: false,
        }
    }
}

/// Generates HTML benchmark reports
pub struct BenchmarkReport {
    config: ReportConfig,
}

impl BenchmarkReport {
    pub fn new(config: ReportConfig) -> Self {
        Self { config }
    }

    /// Generate report from metrics
    pub fn generate(&self, metrics: &BenchmarkMetrics) -> std::io::Result<String> {
        let output_dir = Path::new(&self.config.output_dir);
        fs::create_dir_all(output_dir)?;

        let filename = format!(
            "{}_{}.html",
            metrics.name.replace(' ', "_").to_lowercase(),
            chrono::Utc::now().format("%Y%m%d_%H%M%S")
        );
        let output_path = output_dir.join(&filename);

        let html = self.render_html(metrics);
        fs::write(&output_path, &html)?;

        // Return absolute path for clickable terminal links
        let absolute_path = output_path
            .canonicalize()
            .unwrap_or(output_path);

        Ok(absolute_path.to_string_lossy().to_string())
    }

    fn render_html(&self, metrics: &BenchmarkMetrics) -> String {
        let mut env = Environment::new();
        env.add_template("report", REPORT_TEMPLATE).unwrap();

        let template = env.get_template("report").unwrap();

        // Prepare data for charts
        let schedule_to_start = metrics.schedule_to_start.summary();
        let execution = metrics.execution.summary();
        let end_to_end = metrics.end_to_end.summary();

        let throughput_data = metrics.tasks_completed.timeseries();
        let resource_data = metrics.resources.snapshots();

        // Calculate throughput over time (ops/sec in sliding windows)
        let throughput_series = calculate_throughput_series(&throughput_data);

        template
            .render(context! {
                title => self.config.title,
                benchmark_name => metrics.name,
                duration_secs => metrics.elapsed().as_secs_f64(),
                total_tasks => metrics.tasks_completed.total(),
                throughput => metrics.tasks_completed.throughput(),

                // Latency summaries
                schedule_to_start => format_latency_summary(&schedule_to_start),
                execution => format_latency_summary(&execution),
                end_to_end => format_latency_summary(&end_to_end),

                // Resource usage
                peak_memory_mb => metrics.resources.peak_memory_mb(),
                avg_cpu_percent => metrics.resources.avg_cpu_percent(),

                // Chart data (JSON)
                throughput_chart_data => serde_json::to_string(&throughput_series).unwrap(),
                latency_chart_data => format_latency_chart_data(metrics),
                resource_chart_data => format_resource_chart_data(&resource_data),

                // Latency distribution for histogram
                latency_histogram_data => format_latency_histogram(metrics),
            })
            .unwrap()
    }
}

fn format_latency_summary(summary: &LatencySummary) -> serde_json::Value {
    serde_json::json!({
        "count": summary.count,
        "mean_ms": summary.mean.as_secs_f64() * 1000.0,
        "min_ms": summary.min.as_secs_f64() * 1000.0,
        "max_ms": summary.max.as_secs_f64() * 1000.0,
        "p50_ms": summary.p50.as_secs_f64() * 1000.0,
        "p95_ms": summary.p95.as_secs_f64() * 1000.0,
        "p99_ms": summary.p99.as_secs_f64() * 1000.0,
    })
}

fn calculate_throughput_series(data: &[(u64, u64)]) -> Vec<(f64, f64)> {
    if data.len() < 2 {
        return vec![];
    }

    let mut result = Vec::new();

    for i in 1..data.len() {
        let (t1, c1) = data[i - 1];
        let (t2, c2) = data[i];

        let dt = (t2 - t1) as f64 / 1000.0; // seconds
        if dt > 0.0 {
            let ops_per_sec = (c2 - c1) as f64 / dt;
            result.push((t2 as f64 / 1000.0, ops_per_sec));
        }
    }

    // Smooth with sliding window if we have enough data
    if result.len() > 10 {
        let window = 5;
        let mut smoothed = Vec::new();
        for i in window..result.len() {
            let avg: f64 = result[i - window..i].iter().map(|(_, v)| v).sum::<f64>() / window as f64;
            smoothed.push((result[i].0, avg));
        }
        return smoothed;
    }

    result
}

fn format_latency_chart_data(metrics: &BenchmarkMetrics) -> String {
    let samples = metrics.end_to_end.samples();

    // Sample at most 1000 points for the chart
    let step = (samples.len() / 1000).max(1);
    let data: Vec<(f64, f64)> = samples
        .iter()
        .enumerate()
        .filter(|(i, _)| i % step == 0)
        .map(|(i, d)| (i as f64, d.as_secs_f64() * 1000.0))
        .collect();

    serde_json::to_string(&data).unwrap()
}

fn format_resource_chart_data(snapshots: &[ResourceSnapshot]) -> String {
    let data: Vec<serde_json::Value> = snapshots
        .iter()
        .map(|s| {
            serde_json::json!({
                "time": s.timestamp_ms as f64 / 1000.0,
                "memory": s.memory_rss_mb,
                "cpu": s.cpu_percent,
            })
        })
        .collect();

    serde_json::to_string(&data).unwrap()
}

fn format_latency_histogram(metrics: &BenchmarkMetrics) -> String {
    let samples = metrics.end_to_end.samples();
    if samples.is_empty() {
        return "[]".to_string();
    }

    // Create histogram buckets
    let max_ms = samples.iter().map(|d| d.as_millis()).max().unwrap_or(1) as f64;
    let bucket_count = 50;
    let bucket_size = (max_ms / bucket_count as f64).max(1.0);

    let mut buckets = vec![0u64; bucket_count + 1];
    for sample in &samples {
        let idx = (sample.as_secs_f64() * 1000.0 / bucket_size) as usize;
        let idx = idx.min(bucket_count);
        buckets[idx] += 1;
    }

    let data: Vec<serde_json::Value> = buckets
        .iter()
        .enumerate()
        .map(|(i, count)| {
            serde_json::json!({
                "bucket": format!("{:.1}", i as f64 * bucket_size),
                "count": count,
            })
        })
        .collect();

    serde_json::to_string(&data).unwrap()
}

const REPORT_TEMPLATE: &str = r##"
<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>{{ title }} - {{ benchmark_name }}</title>
    <script src="https://cdn.jsdelivr.net/npm/chart.js"></script>
    <style>
        :root {
            --bg-primary: #1a1a2e;
            --bg-secondary: #16213e;
            --bg-card: #1f2940;
            --text-primary: #eee;
            --text-secondary: #888;
            --accent: #0f3460;
            --success: #00d26a;
            --warning: #f39c12;
            --danger: #e74c3c;
            --blue: #3498db;
            --purple: #9b59b6;
        }

        * {
            margin: 0;
            padding: 0;
            box-sizing: border-box;
        }

        body {
            font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
            background: var(--bg-primary);
            color: var(--text-primary);
            line-height: 1.6;
        }

        .container {
            max-width: 1400px;
            margin: 0 auto;
            padding: 20px;
        }

        header {
            background: var(--bg-secondary);
            padding: 30px;
            margin-bottom: 30px;
            border-radius: 10px;
        }

        h1 {
            font-size: 2rem;
            margin-bottom: 10px;
        }

        .subtitle {
            color: var(--text-secondary);
            font-size: 1.1rem;
        }

        .stats-grid {
            display: grid;
            grid-template-columns: repeat(auto-fit, minmax(200px, 1fr));
            gap: 20px;
            margin-bottom: 30px;
        }

        .stat-card {
            background: var(--bg-card);
            padding: 20px;
            border-radius: 10px;
            text-align: center;
        }

        .stat-value {
            font-size: 2rem;
            font-weight: bold;
            color: var(--success);
        }

        .stat-label {
            color: var(--text-secondary);
            font-size: 0.9rem;
            margin-top: 5px;
        }

        .chart-container {
            background: var(--bg-card);
            padding: 20px;
            border-radius: 10px;
            margin-bottom: 20px;
        }

        .chart-title {
            font-size: 1.2rem;
            margin-bottom: 15px;
            color: var(--text-primary);
        }

        .charts-row {
            display: grid;
            grid-template-columns: repeat(auto-fit, minmax(500px, 1fr));
            gap: 20px;
            margin-bottom: 20px;
        }

        table {
            width: 100%;
            border-collapse: collapse;
            margin-top: 10px;
        }

        th, td {
            padding: 12px;
            text-align: left;
            border-bottom: 1px solid var(--accent);
        }

        th {
            color: var(--text-secondary);
            font-weight: normal;
        }

        td {
            font-family: monospace;
        }

        .latency-table {
            background: var(--bg-card);
            padding: 20px;
            border-radius: 10px;
            margin-bottom: 20px;
        }

        .percentile-good { color: var(--success); }
        .percentile-warning { color: var(--warning); }
        .percentile-danger { color: var(--danger); }

        canvas {
            max-height: 300px;
        }
    </style>
</head>
<body>
    <div class="container">
        <header>
            <h1>{{ title }}</h1>
            <div class="subtitle">{{ benchmark_name }} • Duration: {{ "%.2f"|format(duration_secs) }}s</div>
        </header>

        <div class="stats-grid">
            <div class="stat-card">
                <div class="stat-value">{{ total_tasks }}</div>
                <div class="stat-label">Total Tasks</div>
            </div>
            <div class="stat-card">
                <div class="stat-value">{{ "%.1f"|format(throughput) }}</div>
                <div class="stat-label">Tasks/sec</div>
            </div>
            <div class="stat-card">
                <div class="stat-value">{{ "%.1f"|format(end_to_end.p50_ms) }}ms</div>
                <div class="stat-label">P50 Latency</div>
            </div>
            <div class="stat-card">
                <div class="stat-value">{{ "%.1f"|format(end_to_end.p99_ms) }}ms</div>
                <div class="stat-label">P99 Latency</div>
            </div>
            <div class="stat-card">
                <div class="stat-value">{{ "%.1f"|format(peak_memory_mb) }}MB</div>
                <div class="stat-label">Peak Memory</div>
            </div>
            <div class="stat-card">
                <div class="stat-value">{{ "%.1f"|format(avg_cpu_percent) }}%</div>
                <div class="stat-label">Avg CPU</div>
            </div>
        </div>

        <div class="charts-row">
            <div class="chart-container">
                <div class="chart-title">Throughput Over Time</div>
                <canvas id="throughputChart"></canvas>
            </div>
            <div class="chart-container">
                <div class="chart-title">Latency Distribution</div>
                <canvas id="latencyHistogram"></canvas>
            </div>
        </div>

        <div class="charts-row">
            <div class="chart-container">
                <div class="chart-title">Resource Usage</div>
                <canvas id="resourceChart"></canvas>
            </div>
            <div class="chart-container">
                <div class="chart-title">Latency Over Time</div>
                <canvas id="latencyTimeChart"></canvas>
            </div>
        </div>

        <div class="latency-table">
            <div class="chart-title">Latency Statistics (ms)</div>
            <table>
                <thead>
                    <tr>
                        <th>Metric</th>
                        <th>Count</th>
                        <th>Mean</th>
                        <th>Min</th>
                        <th>P50</th>
                        <th>P95</th>
                        <th>P99</th>
                        <th>Max</th>
                    </tr>
                </thead>
                <tbody>
                    <tr>
                        <td>Schedule → Start</td>
                        <td>{{ schedule_to_start.count }}</td>
                        <td>{{ "%.2f"|format(schedule_to_start.mean_ms) }}</td>
                        <td>{{ "%.2f"|format(schedule_to_start.min_ms) }}</td>
                        <td>{{ "%.2f"|format(schedule_to_start.p50_ms) }}</td>
                        <td>{{ "%.2f"|format(schedule_to_start.p95_ms) }}</td>
                        <td>{{ "%.2f"|format(schedule_to_start.p99_ms) }}</td>
                        <td>{{ "%.2f"|format(schedule_to_start.max_ms) }}</td>
                    </tr>
                    <tr>
                        <td>Execution</td>
                        <td>{{ execution.count }}</td>
                        <td>{{ "%.2f"|format(execution.mean_ms) }}</td>
                        <td>{{ "%.2f"|format(execution.min_ms) }}</td>
                        <td>{{ "%.2f"|format(execution.p50_ms) }}</td>
                        <td>{{ "%.2f"|format(execution.p95_ms) }}</td>
                        <td>{{ "%.2f"|format(execution.p99_ms) }}</td>
                        <td>{{ "%.2f"|format(execution.max_ms) }}</td>
                    </tr>
                    <tr>
                        <td>End-to-End</td>
                        <td>{{ end_to_end.count }}</td>
                        <td>{{ "%.2f"|format(end_to_end.mean_ms) }}</td>
                        <td>{{ "%.2f"|format(end_to_end.min_ms) }}</td>
                        <td class="percentile-good">{{ "%.2f"|format(end_to_end.p50_ms) }}</td>
                        <td class="percentile-warning">{{ "%.2f"|format(end_to_end.p95_ms) }}</td>
                        <td class="percentile-danger">{{ "%.2f"|format(end_to_end.p99_ms) }}</td>
                        <td>{{ "%.2f"|format(end_to_end.max_ms) }}</td>
                    </tr>
                </tbody>
            </table>
        </div>
    </div>

    <script>
        const chartColors = {
            blue: 'rgb(52, 152, 219)',
            green: 'rgb(0, 210, 106)',
            purple: 'rgb(155, 89, 182)',
            orange: 'rgb(243, 156, 18)',
            red: 'rgb(231, 76, 60)',
        };

        // Throughput Chart
        const throughputData = {{ throughput_chart_data|safe }};
        new Chart(document.getElementById('throughputChart'), {
            type: 'line',
            data: {
                labels: throughputData.map(d => d[0].toFixed(1) + 's'),
                datasets: [{
                    label: 'Tasks/sec',
                    data: throughputData.map(d => d[1]),
                    borderColor: chartColors.green,
                    backgroundColor: 'rgba(0, 210, 106, 0.1)',
                    fill: true,
                    tension: 0.4,
                }]
            },
            options: {
                responsive: true,
                plugins: { legend: { display: false } },
                scales: {
                    x: { grid: { color: 'rgba(255,255,255,0.1)' } },
                    y: { grid: { color: 'rgba(255,255,255,0.1)' }, beginAtZero: true }
                }
            }
        });

        // Latency Histogram
        const histogramData = {{ latency_histogram_data|safe }};
        new Chart(document.getElementById('latencyHistogram'), {
            type: 'bar',
            data: {
                labels: histogramData.map(d => d.bucket + 'ms'),
                datasets: [{
                    label: 'Count',
                    data: histogramData.map(d => d.count),
                    backgroundColor: chartColors.blue,
                }]
            },
            options: {
                responsive: true,
                plugins: { legend: { display: false } },
                scales: {
                    x: { grid: { display: false } },
                    y: { grid: { color: 'rgba(255,255,255,0.1)' }, beginAtZero: true }
                }
            }
        });

        // Resource Chart
        const resourceData = {{ resource_chart_data|safe }};
        new Chart(document.getElementById('resourceChart'), {
            type: 'line',
            data: {
                labels: resourceData.map(d => d.time.toFixed(1) + 's'),
                datasets: [{
                    label: 'Memory (MB)',
                    data: resourceData.map(d => d.memory),
                    borderColor: chartColors.purple,
                    yAxisID: 'y',
                }, {
                    label: 'CPU (%)',
                    data: resourceData.map(d => d.cpu),
                    borderColor: chartColors.orange,
                    yAxisID: 'y1',
                }]
            },
            options: {
                responsive: true,
                scales: {
                    x: { grid: { color: 'rgba(255,255,255,0.1)' } },
                    y: {
                        type: 'linear',
                        position: 'left',
                        grid: { color: 'rgba(255,255,255,0.1)' },
                        title: { display: true, text: 'Memory (MB)' }
                    },
                    y1: {
                        type: 'linear',
                        position: 'right',
                        grid: { display: false },
                        title: { display: true, text: 'CPU (%)' }
                    }
                }
            }
        });

        // Latency Over Time
        const latencyData = {{ latency_chart_data|safe }};
        new Chart(document.getElementById('latencyTimeChart'), {
            type: 'scatter',
            data: {
                datasets: [{
                    label: 'Latency (ms)',
                    data: latencyData.map(d => ({ x: d[0], y: d[1] })),
                    backgroundColor: 'rgba(52, 152, 219, 0.5)',
                    pointRadius: 2,
                }]
            },
            options: {
                responsive: true,
                plugins: { legend: { display: false } },
                scales: {
                    x: {
                        grid: { color: 'rgba(255,255,255,0.1)' },
                        title: { display: true, text: 'Request #' }
                    },
                    y: {
                        grid: { color: 'rgba(255,255,255,0.1)' },
                        title: { display: true, text: 'Latency (ms)' }
                    }
                }
            }
        });
    </script>
</body>
</html>
"##;
