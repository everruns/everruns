# Durable Execution Benchmarks

Load tests for the durable execution engine with HTML reports and checkpoint-based historical comparison.

## Quick Start

```bash
# Using convenience scripts (recommended)
./scripts/dev.sh durable-bench         # Run all benchmarks
./scripts/dev.sh durable-bench-save    # Run and save checkpoints
./scripts/dev.sh durable-bench-save ci-4cpu-8gb  # With custom moniker

# Or run individual benchmarks directly
cargo bench -p everruns-durable --bench concurrent_workers
cargo bench -p everruns-durable --bench workflow_throughput

# With checkpoint saving
cargo bench -p everruns-durable --bench concurrent_workers -- --save
cargo bench -p everruns-durable --bench workflow_throughput -- --save

# With custom environment moniker (e.g., for CI)
cargo bench -p everruns-durable --bench concurrent_workers -- --save --moniker ci-4cpu-8gb
cargo bench -p everruns-durable --bench workflow_throughput -- --save --moniker ci-4cpu-8gb
```

## Available Benchmarks

### concurrent_workers

Tests task scheduling performance with varying worker counts:

| Scenario | Tasks | Workers | Execution |
|----------|-------|---------|-----------|
| baseline_1_worker | 10k | 1 | None |
| scale_10_workers | 10k | 10 | None |
| scale_50_workers | 10k | 50 | None |
| scale_100_workers | 10k | 100 | None |
| realistic_10_workers | 1k | 10 | Simulated I/O |
| realistic_50_workers | 1k | 50 | Simulated I/O |
| realistic_100_workers | 1k | 100 | Simulated I/O |
| burst_50k_tasks | 50k | 100 | None |

### workflow_throughput

Tests multi-step workflow execution with sequential activities:

| Scenario | Workflows | Steps | Workers | Total Tasks |
|----------|-----------|-------|---------|-------------|
| small_10wf_10steps | 10 | 10 | 10 | 100 |
| medium_100wf_50steps | 100 | 50 | 50 | 5,000 |
| target_1000wf_100steps | 1,000 | 100 | 100 | 100,000 |
| target_1000wf_100steps_exec | 1,000 | 100 | 100 | 100,000 |
| parallel_5000wf_20steps | 5,000 | 20 | 200 | 100,000 |
| deep_100wf_500steps | 100 | 500 | 50 | 50,000 |

Supports `--save` and `--moniker` flags for checkpointing.

Both benchmarks measure **Schedule-to-Start (S2S) latency** which tracks how fast tasks are claimed under load. This provides realistic task claiming performance metrics including P50, P95, and P99 percentiles.

## Output

### HTML Reports

Generated in `target/benchmark-reports/`:
- Interactive charts (throughput, latency distribution, resource usage)
- Percentile statistics table
- Metrics glossary with interpretation guidance

### Checkpoints

Saved to `target/benchmark-checkpoints/` when using `--save`:
- JSON files with full metrics snapshot
- Environment info (OS, CPU, memory, moniker)
- Automatic comparison with previous runs

## Checkpointing

### Environment Monikers

Auto-detected monikers based on hardware:
- Apple Silicon: `local-M4-Pro-48GB`
- Intel: `local-i9-12900K-64GB`
- AMD: `local-R9-5950X-128GB`
- Generic: `local-8c-32GB`

Custom monikers for CI/cloud environments:
```bash
cargo bench ... -- --save --moniker ci-github-4cpu-8gb
cargo bench ... -- --save --moniker aws-c5.xlarge
```

### Historical Comparison

When checkpoints exist, you'll see comparison output:
```
📊 Historical comparison (vs last run):

   baseline_1_worker
      Throughput: 15234.5 → 15890.2 (+4.3%)
      E2E P99:    2.45ms → 2.32ms (-5.3%)

   burst_50k_tasks
      Throughput: 12567.8 → 12890.1 (+2.6%)
      E2E P99:    5.67ms → 5.45ms (-3.9%)
```

### Programmatic Access

```rust
use everruns_durable::bench::{CheckpointStore, CheckpointComparison};

let store = CheckpointStore::default_location();

// List all checkpoints
let checkpoints = store.list(None)?;

// Get checkpoints for comparison
let history = store.get_comparison("baseline_1_worker", Some("local-M4-48GB"), 5)?;

// Compare two checkpoints
let comparison = CheckpointComparison::compare(&current, &baseline);
if comparison.has_regression(10.0) {
    println!("Performance regression detected!");
}

// Cleanup old checkpoints (keep 10 per benchmark+moniker)
store.cleanup(10)?;
```

## Key Metrics

| Metric | Description | Target |
|--------|-------------|--------|
| Throughput | Tasks/sec sustained | Higher is better |
| S2S P99 | Schedule-to-start latency | < 10ms |
| E2E P50 | Median end-to-end latency | Lower is better |
| E2E P99 | Tail latency (1% worst) | < 3x P50 |

## Interpreting Results

- **High S2S latency**: Add more workers or increase batch claim size
- **High E2E with low S2S**: Execution or completion overhead
- **P99 >> P50**: High variance, check for contention or GC pauses
- **Throughput plateau**: Hit CPU or I/O bottleneck
