//! Everruns Load Test
//!
//! Uses everruns-sdk for agent/message operations. Raw reqwest for:
//! - Session creation (SDK lacks harness_id field)
//! - Event polling (SDK lacks offset/limit pagination)
//!
//! Usage:
//!   cargo bench --package everruns-server --bench load_test
//!   # Or via just:
//!   just load-test
//!
//! Configuration via environment:
//!   API_URL=http://localhost:9300/api   # API endpoint
//!   SESSIONS=100                    # Number of parallel sessions
//!   MESSAGES_PER_SESSION=50         # Messages per session
//!   MODEL_ID=llmsim                 # Model to use (llmsim, llmsim-ttft-500, etc.)
//!   TARGET=docker-example           # Target label (auto-detected if unset)

use std::fs;
use std::io::{BufReader, BufWriter};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use everruns_sdk::{CreateAgentRequest, Everruns};
use futures::stream::{self, StreamExt};
use parking_lot::Mutex;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use sysinfo::{CpuRefreshKind, MemoryRefreshKind, RefreshKind, System};
use tokio::sync::Semaphore;

/// Seed harness ID from seed.rs (DEFAULT_HARNESS = 0x01933b5a_0000_7000_8000_000000000601)
const DEFAULT_HARNESS_ID: &str = "harness_01933b5a000070008000000000000601";

/// Seed llmsim model ID from seed.rs (LLMSIM_DEFAULT = 0x01933b5a_0000_7000_8000_000000000401)
const DEFAULT_LLMSIM_MODEL_ID: &str = "model_01933b5a000070008000000000000401";

// ============================================================================
// Configuration
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LoadTestConfig {
    api_url: String,
    sessions: usize,
    messages_per_session: usize,
    model_id: String,
    max_concurrent_sessions: usize,
    timeout_secs: u64,
    #[serde(default = "default_bench_name")]
    bench_name: String,
}

impl Default for LoadTestConfig {
    fn default() -> Self {
        Self {
            api_url: std::env::var("API_URL")
                .unwrap_or_else(|_| "http://localhost:9300/api".to_string()),
            sessions: std::env::var("SESSIONS")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(100),
            messages_per_session: std::env::var("MESSAGES_PER_SESSION")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(50),
            model_id: std::env::var("MODEL_ID")
                .unwrap_or_else(|_| DEFAULT_LLMSIM_MODEL_ID.to_string()),
            max_concurrent_sessions: std::env::var("MAX_CONCURRENT")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(50),
            timeout_secs: std::env::var("TIMEOUT_SECS")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(300),
            bench_name: "throughput".to_string(),
        }
    }
}

// ============================================================================
// CLI Args
// ============================================================================

struct CliArgs {
    save: bool,
    moniker: Option<String>,
    bench_name: Option<String>,
    help: bool,
}

impl CliArgs {
    fn parse() -> Self {
        let args: Vec<String> = std::env::args().collect();

        let help = args.iter().any(|a| a == "--help" || a == "-h");
        let save = args.iter().any(|a| a == "--save");

        let moniker = args
            .iter()
            .position(|a| a == "--moniker")
            .and_then(|i| args.get(i + 1))
            .cloned();

        let bench_name = args
            .iter()
            .position(|a| a == "--bench-name")
            .and_then(|i| args.get(i + 1))
            .cloned();

        Self {
            save,
            moniker,
            bench_name,
            help,
        }
    }
}

// ============================================================================
// Checkpoint Types
// ============================================================================

/// Environment information for a benchmark run
#[derive(Debug, Clone, Serialize, Deserialize)]
struct EnvironmentInfo {
    /// Human-readable moniker (e.g., "local-M4-46GB", "ci-4cpu-8gb")
    moniker: String,
    /// Operating system name and version
    os: String,
    /// CPU model/name
    cpu_name: String,
    /// Number of CPU cores
    cpu_cores: usize,
    /// Total system memory in GB
    memory_gb: f64,
    /// Hostname (optional)
    hostname: Option<String>,
}

impl EnvironmentInfo {
    /// Detect environment information automatically
    fn detect() -> Self {
        let system = System::new_with_specifics(
            RefreshKind::nothing()
                .with_cpu(CpuRefreshKind::everything())
                .with_memory(MemoryRefreshKind::everything()),
        );

        let cpu_name = system
            .cpus()
            .first()
            .map(|cpu| cpu.brand().to_string())
            .unwrap_or_else(|| "Unknown CPU".to_string());

        let cpu_cores = system.cpus().len();
        let memory_gb = system.total_memory() as f64 / (1024.0 * 1024.0 * 1024.0);

        let os = format!(
            "{} {}",
            System::name().unwrap_or_default(),
            System::os_version().unwrap_or_default()
        );
        let hostname = System::host_name();

        // Generate default moniker from environment
        let moniker = Self::generate_moniker(&cpu_name, cpu_cores, memory_gb);

        Self {
            moniker,
            os,
            cpu_name,
            cpu_cores,
            memory_gb,
            hostname,
        }
    }

    /// Detect with custom moniker
    fn detect_with_moniker(moniker: impl Into<String>) -> Self {
        let mut info = Self::detect();
        info.moniker = moniker.into();
        info
    }

    /// Generate a default moniker from CPU/memory info
    fn generate_moniker(cpu_name: &str, cores: usize, memory_gb: f64) -> String {
        // Extract short CPU identifier
        let cpu_short = if cpu_name.contains("Apple") {
            cpu_name
                .replace("Apple ", "")
                .split_whitespace()
                .collect::<Vec<_>>()
                .join("-")
        } else if cpu_name.contains("Intel") {
            cpu_name
                .split_whitespace()
                .find(|s| s.starts_with('i') && s.contains('-'))
                .unwrap_or("Intel")
                .to_string()
        } else if cpu_name.contains("AMD") {
            let parts: Vec<&str> = cpu_name.split_whitespace().collect();
            if parts.len() >= 4 {
                format!(
                    "R{}-{}",
                    parts.get(2).unwrap_or(&""),
                    parts.get(3).unwrap_or(&"")
                )
            } else {
                "AMD".to_string()
            }
        } else {
            format!("{}c", cores)
        };

        format!("local-{}-{}GB", cpu_short, memory_gb.round() as u64)
    }
}

/// Server information fetched from health endpoints before the test
#[derive(Debug, Clone, Serialize, Deserialize)]
struct ServerInfo {
    /// Server version (from GET /health)
    version: String,
    /// Auth mode (from GET /health)
    auth_mode: String,
    /// Target label (from TARGET env var or auto-inferred)
    target: String,
    /// Total registered workers (from GET /v1/durable/health)
    workers: usize,
    /// Currently active workers
    active_workers: usize,
    /// Total task capacity across all workers
    total_capacity: usize,
}

impl ServerInfo {
    /// Fetch server info from health endpoints.
    async fn fetch(http: &Client, api_url: &str) -> Self {
        // Derive base URL: /api -> /health (sibling, not child)
        let base_url = api_url.trim_end_matches('/');
        let health_url = if let Some(prefix) = base_url.strip_suffix("/api") {
            format!("{}/health", prefix)
        } else {
            format!("{}/health", base_url)
        };

        // GET /health -> version, auth_mode
        let (version, auth_mode) = match http.get(&health_url).send().await {
            Ok(resp) => {
                if let Ok(body) = resp.json::<serde_json::Value>().await {
                    (
                        body["version"].as_str().unwrap_or("unknown").to_string(),
                        body["auth_mode"].as_str().unwrap_or("unknown").to_string(),
                    )
                } else {
                    ("unknown".to_string(), "unknown".to_string())
                }
            }
            Err(_) => ("unknown".to_string(), "unknown".to_string()),
        };

        // GET /v1/durable/health -> worker topology
        let durable_url = format!("{}/v1/durable/health", base_url);
        let (workers, active_workers, total_capacity) =
            match http.get(&durable_url).send().await {
                Ok(resp) => {
                    if let Ok(body) = resp.json::<serde_json::Value>().await {
                        (
                            body["total_workers"].as_u64().unwrap_or(0) as usize,
                            body["active_workers"].as_u64().unwrap_or(0) as usize,
                            body["total_capacity"].as_u64().unwrap_or(0) as usize,
                        )
                    } else {
                        (0, 0, 0)
                    }
                }
                Err(_) => (0, 0, 0),
            };

        // Target: explicit env var or auto-inferred from URL + worker count
        let target = std::env::var("TARGET").unwrap_or_else(|_| {
            let is_local = api_url.contains("localhost") || api_url.contains("127.0.0.1");
            let location = if is_local { "local" } else { "remote" };
            format!("{}-{}w", location, workers)
        });

        Self {
            version,
            auth_mode,
            target,
            workers,
            active_workers,
            total_capacity,
        }
    }
}

/// Serializable metrics for checkpointing
#[derive(Debug, Clone, Serialize, Deserialize)]
struct MetricsSnapshot {
    sessions_created: u64,
    sessions_completed: u64,
    sessions_failed: u64,
    messages_sent: u64,
    messages_completed: u64,
    messages_failed: u64,
    latency_p50_ms: f64,
    latency_p95_ms: f64,
    latency_p99_ms: f64,
    latency_avg_ms: f64,
    throughput_msg_per_sec: f64,
    duration_secs: f64,
    error_count: usize,
}

/// A load test checkpoint for saving results
#[derive(Debug, Clone, Serialize, Deserialize)]
struct LoadTestCheckpoint {
    /// Unique ID for this checkpoint
    id: String,
    /// Bench name (e.g., "throughput", "history-depth", "horizontal-scale")
    #[serde(default = "default_bench_name")]
    bench_name: String,
    /// Timestamp when test was run
    timestamp: DateTime<Utc>,
    /// Server information (version, target, workers)
    #[serde(default)]
    server: Option<ServerInfo>,
    /// Environment information
    environment: EnvironmentInfo,
    /// Test configuration
    config: LoadTestConfig,
    /// Metrics snapshot
    metrics: MetricsSnapshot,
    /// Sample errors (first 10)
    errors: Vec<String>,
}

fn default_bench_name() -> String {
    "throughput".to_string()
}

impl LoadTestCheckpoint {
    fn new(
        bench_name: String,
        server: ServerInfo,
        environment: EnvironmentInfo,
        config: LoadTestConfig,
        metrics: MetricsSnapshot,
        errors: Vec<String>,
    ) -> Self {
        Self {
            id: uuid::Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext)).to_string(),
            bench_name,
            timestamp: Utc::now(),
            server: Some(server),
            environment,
            config,
            metrics,
            errors,
        }
    }

    fn filename(&self) -> String {
        format!(
            "{}_{}_{}_{}.json",
            self.bench_name,
            self.config.sessions,
            self.environment.moniker.replace(' ', "_").to_lowercase(),
            self.timestamp.format("%Y%m%d_%H%M%S"),
        )
    }
}

/// Manages checkpoint storage
struct CheckpointStore {
    directory: PathBuf,
}

impl CheckpointStore {
    fn new() -> Self {
        Self {
            directory: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("benches/checkpoints"),
        }
    }

    fn save(&self, checkpoint: &LoadTestCheckpoint) -> std::io::Result<PathBuf> {
        fs::create_dir_all(&self.directory)?;

        let path = self.directory.join(checkpoint.filename());
        let file = fs::File::create(&path)?;
        let writer = BufWriter::new(file);
        serde_json::to_writer_pretty(writer, checkpoint)?;

        let absolute = path.canonicalize().unwrap_or(path);
        Ok(absolute)
    }

    fn list_recent(&self, limit: usize) -> std::io::Result<Vec<LoadTestCheckpoint>> {
        if !self.directory.exists() {
            return Ok(Vec::new());
        }

        let mut checkpoints = Vec::new();

        for entry in fs::read_dir(&self.directory)? {
            let entry = entry?;
            let path = entry.path();

            if path.extension().map(|e| e == "json").unwrap_or(false)
                && let Ok(file) = fs::File::open(&path)
            {
                let reader = BufReader::new(file);
                if let Ok(checkpoint) = serde_json::from_reader::<_, LoadTestCheckpoint>(reader) {
                    checkpoints.push(checkpoint);
                }
            }
        }

        // Sort by timestamp (newest first)
        checkpoints.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
        checkpoints.truncate(limit);

        Ok(checkpoints)
    }
}

// ============================================================================
// API Types (for raw reqwest calls where SDK lacks support)
// ============================================================================

/// Session creation requires harness_id which the SDK doesn't support yet
#[derive(Debug, Serialize)]
struct CreateSessionRequest {
    harness_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    agent_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    model_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Session {
    id: String,
    #[serde(default)]
    status: String,
}

#[derive(Debug, Deserialize)]
struct MessageResponse {
    role: String,
}

#[derive(Debug, Deserialize)]
struct MessagesListResponse {
    data: Vec<MessageResponse>,
}

// ============================================================================
// Metrics
// ============================================================================

#[derive(Default)]
struct Metrics {
    sessions_created: AtomicU64,
    sessions_completed: AtomicU64,
    sessions_failed: AtomicU64,
    messages_sent: AtomicU64,
    messages_completed: AtomicU64,
    messages_failed: AtomicU64,
    latencies: Mutex<Vec<Duration>>,
    errors: Mutex<Vec<String>>,
}

impl Metrics {
    fn record_latency(&self, duration: Duration) {
        self.latencies.lock().push(duration);
    }

    fn record_error(&self, error: String) {
        let mut errors = self.errors.lock();
        if errors.len() < 100 {
            errors.push(error);
        }
    }

    fn summary(&self, duration: Duration) -> MetricsSummary {
        let mut latencies = self.latencies.lock().clone();
        latencies.sort();

        let p50 = latencies
            .get(latencies.len() / 2)
            .copied()
            .unwrap_or_default();
        let p95 = latencies
            .get(latencies.len() * 95 / 100)
            .copied()
            .unwrap_or_default();
        let p99 = latencies
            .get(latencies.len() * 99 / 100)
            .copied()
            .unwrap_or_default();
        let avg = if latencies.is_empty() {
            Duration::ZERO
        } else {
            latencies.iter().sum::<Duration>() / latencies.len() as u32
        };

        let messages_completed = self.messages_completed.load(Ordering::Relaxed);
        let throughput = messages_completed as f64 / duration.as_secs_f64();

        MetricsSummary {
            sessions_created: self.sessions_created.load(Ordering::Relaxed),
            sessions_completed: self.sessions_completed.load(Ordering::Relaxed),
            sessions_failed: self.sessions_failed.load(Ordering::Relaxed),
            messages_sent: self.messages_sent.load(Ordering::Relaxed),
            messages_completed,
            messages_failed: self.messages_failed.load(Ordering::Relaxed),
            latency_p50: p50,
            latency_p95: p95,
            latency_p99: p99,
            latency_avg: avg,
            throughput,
            duration,
            errors: self.errors.lock().clone(),
        }
    }
}

struct MetricsSummary {
    sessions_created: u64,
    sessions_completed: u64,
    sessions_failed: u64,
    messages_sent: u64,
    messages_completed: u64,
    messages_failed: u64,
    latency_p50: Duration,
    latency_p95: Duration,
    latency_p99: Duration,
    latency_avg: Duration,
    throughput: f64,
    duration: Duration,
    errors: Vec<String>,
}

impl MetricsSummary {
    fn to_snapshot(&self) -> MetricsSnapshot {
        MetricsSnapshot {
            sessions_created: self.sessions_created,
            sessions_completed: self.sessions_completed,
            sessions_failed: self.sessions_failed,
            messages_sent: self.messages_sent,
            messages_completed: self.messages_completed,
            messages_failed: self.messages_failed,
            latency_p50_ms: self.latency_p50.as_secs_f64() * 1000.0,
            latency_p95_ms: self.latency_p95.as_secs_f64() * 1000.0,
            latency_p99_ms: self.latency_p99.as_secs_f64() * 1000.0,
            latency_avg_ms: self.latency_avg.as_secs_f64() * 1000.0,
            throughput_msg_per_sec: self.throughput,
            duration_secs: self.duration.as_secs_f64(),
            error_count: self.errors.len(),
        }
    }
}

// ============================================================================
// Load Test Runner
// ============================================================================

struct LoadTestRunner {
    config: LoadTestConfig,
    /// SDK client for agent/message operations
    sdk: Everruns,
    /// Raw reqwest for session creation (needs harness_id) and event polling (needs pagination)
    http: Client,
    metrics: Arc<Metrics>,
}

impl LoadTestRunner {
    fn new(config: LoadTestConfig) -> Self {
        let http = Client::builder()
            .timeout(Duration::from_secs(config.timeout_secs))
            .pool_max_idle_per_host(100)
            .build()
            .expect("Failed to create HTTP client");

        // SDK client - uses dummy API key (AUTH_MODE=none ignores it)
        let sdk = Everruns::with_base_url("load-test", &config.api_url)
            .expect("Failed to create SDK client");

        Self {
            config,
            sdk,
            http,
            metrics: Arc::new(Metrics::default()),
        }
    }

    /// Create agent via SDK
    async fn create_load_test_agent(&self) -> anyhow::Result<String> {
        let req = CreateAgentRequest::new(
            format!("Load Test Agent {}", chrono::Utc::now().timestamp()),
            "You are a helpful assistant for load testing. Respond concisely.",
        )
        .default_model_id(&self.config.model_id);

        let agent = self.sdk.agents().create_with_options(req).await?;
        Ok(agent.id)
    }

    /// Create session via raw reqwest (SDK lacks harness_id support)
    async fn create_session(&self, agent_id: &str, session_num: usize) -> anyhow::Result<String> {
        let url = format!("{}/v1/sessions", self.config.api_url);

        let req = CreateSessionRequest {
            harness_id: DEFAULT_HARNESS_ID.to_string(),
            agent_id: Some(agent_id.to_string()),
            title: Some(format!("Load Test Session {}", session_num)),
            model_id: Some(self.config.model_id.clone()),
        };

        let resp = self
            .http
            .post(&url)
            .json(&req)
            .send()
            .await?
            .error_for_status()?
            .json::<Session>()
            .await?;

        self.metrics
            .sessions_created
            .fetch_add(1, Ordering::Relaxed);
        Ok(resp.id)
    }

    /// Send message and wait for the agent's response to appear.
    /// Waits for session to be idle before sending to ensure the server
    /// creates a new turn (it won't if the session is still active).
    async fn send_message(
        &self,
        session_id: &str,
        message_num: usize,
        expected_agent_count: usize,
    ) -> anyhow::Result<Duration> {
        // Ensure session is idle before sending (server ignores messages to active sessions)
        if message_num > 0 {
            self.wait_for_session_idle(session_id).await?;
        }

        let start = Instant::now();

        let text = format!(
            "Load test message {} of {}. Please respond briefly.",
            message_num + 1,
            self.config.messages_per_session
        );

        self.metrics.messages_sent.fetch_add(1, Ordering::Relaxed);

        let url = format!("{}/v1/sessions/{}/messages", self.config.api_url, session_id);
        let body = serde_json::json!({
            "message": {
                "role": "user",
                "content": [{ "type": "text", "text": text }]
            }
        });
        self.http
            .post(&url)
            .json(&body)
            .send()
            .await?
            .error_for_status()?;

        // Wait until the agent's response message appears
        self.wait_for_agent_response(session_id, expected_agent_count)
            .await?;

        let duration = start.elapsed();
        self.metrics.record_latency(duration);
        self.metrics
            .messages_completed
            .fetch_add(1, Ordering::Relaxed);

        Ok(duration)
    }

    /// Poll session status until idle.
    async fn wait_for_session_idle(&self, session_id: &str) -> anyhow::Result<()> {
        let url = format!("{}/v1/sessions/{}", self.config.api_url, session_id);
        let timeout = Duration::from_secs(60);
        let start = Instant::now();

        loop {
            if start.elapsed() > timeout {
                return Err(anyhow::anyhow!("Timeout waiting for session idle"));
            }

            let session: Session = self
                .http
                .get(&url)
                .send()
                .await?
                .error_for_status()?
                .json()
                .await?;

            if session.status == "idle" {
                return Ok(());
            }

            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    /// Poll messages endpoint until we see the expected number of agent messages.
    async fn wait_for_agent_response(
        &self,
        session_id: &str,
        expected_agent_count: usize,
    ) -> anyhow::Result<()> {
        let url = format!("{}/v1/sessions/{}/messages", self.config.api_url, session_id);
        let timeout = Duration::from_secs(60);
        let start = Instant::now();

        loop {
            if start.elapsed() > timeout {
                return Err(anyhow::anyhow!("Timeout waiting for agent response"));
            }

            let resp: MessagesListResponse = self
                .http
                .get(&url)
                .send()
                .await?
                .error_for_status()?
                .json()
                .await?;

            let agent_count = resp.data.iter().filter(|m| m.role == "agent").count();
            if agent_count >= expected_agent_count {
                return Ok(());
            }

            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    async fn run_session(&self, agent_id: &str, session_num: usize) -> anyhow::Result<()> {
        // Create session
        let session_id = match self.create_session(agent_id, session_num).await {
            Ok(id) => id,
            Err(e) => {
                self.metrics.sessions_failed.fetch_add(1, Ordering::Relaxed);
                self.metrics
                    .record_error(format!("Session {} create failed: {}", session_num, e));
                return Err(e);
            }
        };

        // Send messages sequentially — each waits for agent response before next
        for msg_num in 0..self.config.messages_per_session {
            let expected_agent_count = msg_num + 1;
            match self
                .send_message(&session_id, msg_num, expected_agent_count)
                .await
            {
                Ok(_duration) => {}
                Err(e) => {
                    self.metrics.messages_failed.fetch_add(1, Ordering::Relaxed);
                    self.metrics.record_error(format!(
                        "Session {} message {} failed: {}",
                        session_num, msg_num, e
                    ));
                    // Continue with next message
                }
            }
        }

        self.metrics
            .sessions_completed
            .fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    /// Fetch server info from health endpoints
    async fn fetch_server_info(&self) -> ServerInfo {
        ServerInfo::fetch(&self.http, &self.config.api_url).await
    }

    async fn run(&self) -> anyhow::Result<(MetricsSummary, ServerInfo)> {
        // Fetch server info before the test
        let server_info = self.fetch_server_info().await;

        println!("═══════════════════════════════════════════════════════════");
        println!(
            "              Everruns Load Test [{}]",
            self.config.bench_name
        );
        println!("═══════════════════════════════════════════════════════════");
        println!();
        println!("Server:");
        println!("  Version:   {}", server_info.version);
        println!("  Target:    {}", server_info.target);
        println!(
            "  Workers:   {} active / {} total (capacity: {})",
            server_info.active_workers, server_info.workers, server_info.total_capacity
        );
        println!();
        println!("Configuration:");
        println!("  API URL:              {}", self.config.api_url);
        println!("  Sessions:             {}", self.config.sessions);
        println!(
            "  Messages per session: {}",
            self.config.messages_per_session
        );
        println!("  Model ID:             {}", self.config.model_id);
        println!(
            "  Max concurrent:       {}",
            self.config.max_concurrent_sessions
        );
        println!(
            "  Total messages:       {}",
            self.config.sessions * self.config.messages_per_session
        );
        println!("  Harness:              {} (default)", DEFAULT_HARNESS_ID);
        println!();

        // Create agent via SDK
        println!("Creating load test agent...");
        let agent_id = self.create_load_test_agent().await?;
        println!("  Agent ID: {}", agent_id);
        println!();

        // Semaphore for concurrent session limit
        let semaphore = Arc::new(Semaphore::new(self.config.max_concurrent_sessions));

        let start_time = Instant::now();
        let total_messages = self.config.sessions * self.config.messages_per_session;

        println!(
            "Starting load test with {} sessions ({} total messages)...",
            self.config.sessions, total_messages
        );
        println!();

        // Progress reporting task
        let metrics_clone = self.metrics.clone();
        let progress_handle = tokio::spawn(async move {
            let mut last_completed = 0u64;
            loop {
                tokio::time::sleep(Duration::from_secs(5)).await;
                let completed = metrics_clone.messages_completed.load(Ordering::Relaxed);
                let failed = metrics_clone.messages_failed.load(Ordering::Relaxed);
                let sessions_done = metrics_clone.sessions_completed.load(Ordering::Relaxed);
                let rate = (completed - last_completed) as f64 / 5.0;
                last_completed = completed;
                println!(
                    "   Progress: {} messages completed, {} failed, {} sessions done ({:.1} msg/sec)",
                    completed, failed, sessions_done, rate
                );
            }
        });

        // Run sessions in parallel with concurrency limit
        let session_futures: Vec<_> = (0..self.config.sessions)
            .map(|session_num| {
                let runner = self.clone();
                let agent_id = agent_id.clone();
                let semaphore = semaphore.clone();

                async move {
                    let _permit = semaphore.acquire().await.unwrap();
                    runner.run_session(&agent_id, session_num).await
                }
            })
            .collect();

        // Execute all sessions
        stream::iter(session_futures)
            .buffer_unordered(self.config.max_concurrent_sessions)
            .collect::<Vec<_>>()
            .await;

        let total_duration = start_time.elapsed();
        progress_handle.abort();

        // Generate summary
        let summary = self.metrics.summary(total_duration);

        println!();
        println!("═══════════════════════════════════════════════════════════");
        println!("                    Results");
        println!("═══════════════════════════════════════════════════════════");
        println!();
        println!("Duration: {:.2}s", total_duration.as_secs_f64());
        println!();
        println!("Sessions:");
        println!("  Created:   {}", summary.sessions_created);
        println!("  Completed: {}", summary.sessions_completed);
        println!("  Failed:    {}", summary.sessions_failed);
        println!();
        println!("Messages:");
        println!("  Sent:      {}", summary.messages_sent);
        println!("  Completed: {}", summary.messages_completed);
        println!("  Failed:    {}", summary.messages_failed);
        println!("  Throughput: {:.1} msg/sec", summary.throughput);
        println!();
        println!("Latency (message round-trip):");
        println!("  P50: {:.0}ms", summary.latency_p50.as_secs_f64() * 1000.0);
        println!("  P95: {:.0}ms", summary.latency_p95.as_secs_f64() * 1000.0);
        println!("  P99: {:.0}ms", summary.latency_p99.as_secs_f64() * 1000.0);
        println!("  Avg: {:.0}ms", summary.latency_avg.as_secs_f64() * 1000.0);

        if !summary.errors.is_empty() {
            println!();
            println!("Errors (first 10):");
            for (i, error) in summary.errors.iter().take(10).enumerate() {
                println!("  {}. {}", i + 1, error);
            }
            if summary.errors.len() > 10 {
                println!("  ... and {} more", summary.errors.len() - 10);
            }
        }

        println!();
        println!("═══════════════════════════════════════════════════════════");

        Ok((summary, server_info))
    }
}

impl Clone for LoadTestRunner {
    fn clone(&self) -> Self {
        Self {
            config: self.config.clone(),
            sdk: self.sdk.clone(),
            http: self.http.clone(),
            metrics: self.metrics.clone(),
        }
    }
}

// ============================================================================
// Main
// ============================================================================

fn print_help() {
    println!("Everruns Load Test");
    println!();
    println!("Usage: load_test [OPTIONS]");
    println!();
    println!("Options:");
    println!("  --save              Save results to crates/server/benches/checkpoints/");
    println!("  --bench-name <NAME> Bench name (default: throughput)");
    println!("  --moniker <NAME>    Set custom environment moniker for saved results");
    println!("  --help, -h          Show this help message");
    println!();
    println!("Environment Variables:");
    println!("  API_URL              API endpoint (default: http://localhost:9300/api)");
    println!("  SESSIONS             Number of parallel sessions (default: 100)");
    println!("  MESSAGES_PER_SESSION Messages per session (default: 50)");
    println!("  MODEL_ID             Model ID (default: seed llmsim model)");
    println!("  MAX_CONCURRENT       Max concurrent sessions (default: 50)");
    println!("  TIMEOUT_SECS         Request timeout in seconds (default: 300)");
    println!("  TARGET               Target label for saved results (default: auto-detected)");
    println!("                       Examples: dev, docker-example, staging");
    println!();
    println!("Examples:");
    println!("  # Basic load test");
    println!("  just load-test medium");
    println!();
    println!("  # Save results with custom moniker");
    println!("  just load-test medium --save --moniker ci-4cpu-8gb");
    println!();
    println!("  # Load test against docker example stack");
    println!("  TARGET=docker-example just load-test quick --save");
    println!();
    println!("  # Named bench (history depth test)");
    println!("  SESSIONS=1 MESSAGES_PER_SESSION=200 just load-test quick --save --bench-name history-depth");
    println!();
    println!("  # High volume test with save");
    println!("  SESSIONS=500 just load-test heavy --save");
}

fn print_comparison(
    current: &MetricsSnapshot,
    current_server: &ServerInfo,
    previous: &[LoadTestCheckpoint],
) {
    if previous.is_empty() {
        return;
    }

    let baseline = &previous[0];
    let baseline_metrics = &baseline.metrics;

    println!();
    println!(
        "Comparison with previous run ({}):",
        baseline.environment.moniker
    );
    println!(
        "   Previous: {} @ {}",
        baseline.config.sessions * baseline.config.messages_per_session,
        baseline.timestamp.format("%Y-%m-%d %H:%M")
    );

    // Warn if target differs
    if let Some(ref prev_server) = baseline.server {
        if prev_server.target != current_server.target {
            println!(
                "   WARNING: target mismatch! current={}, previous={}",
                current_server.target, prev_server.target
            );
        }
        if prev_server.version != current_server.version {
            println!(
                "   Version: {} -> {}",
                prev_server.version, current_server.version
            );
        }
    }

    let pct_change = |current: f64, baseline: f64| -> f64 {
        if baseline == 0.0 {
            0.0
        } else {
            ((current - baseline) / baseline) * 100.0
        }
    };

    let arrow = |pct: f64, higher_is_better: bool| -> &'static str {
        let is_better = if higher_is_better {
            pct > 0.0
        } else {
            pct < 0.0
        };
        if pct.abs() < 1.0 {
            "~"
        } else if is_better {
            "BETTER"
        } else {
            "WORSE"
        }
    };

    let throughput_pct = pct_change(
        current.throughput_msg_per_sec,
        baseline_metrics.throughput_msg_per_sec,
    );
    let p50_pct = pct_change(current.latency_p50_ms, baseline_metrics.latency_p50_ms);
    let p99_pct = pct_change(current.latency_p99_ms, baseline_metrics.latency_p99_ms);

    println!(
        "   Throughput: {:>8.1} -> {:>8.1} msg/sec ({:+.1}%) {}",
        baseline_metrics.throughput_msg_per_sec,
        current.throughput_msg_per_sec,
        throughput_pct,
        arrow(throughput_pct, true)
    );
    println!(
        "   P50:        {:>8.1} -> {:>8.1} ms       ({:+.1}%) {}",
        baseline_metrics.latency_p50_ms,
        current.latency_p50_ms,
        p50_pct,
        arrow(p50_pct, false)
    );
    println!(
        "   P99:        {:>8.1} -> {:>8.1} ms       ({:+.1}%) {}",
        baseline_metrics.latency_p99_ms,
        current.latency_p99_ms,
        p99_pct,
        arrow(p99_pct, false)
    );
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = CliArgs::parse();

    if cli.help {
        print_help();
        return Ok(());
    }

    let mut config = LoadTestConfig::default();
    if let Some(name) = cli.bench_name {
        config.bench_name = name;
    }
    let runner = LoadTestRunner::new(config.clone());

    let (summary, server_info) = runner.run().await?;

    // Save checkpoint if requested
    if cli.save {
        let environment = match &cli.moniker {
            Some(m) => EnvironmentInfo::detect_with_moniker(m),
            None => EnvironmentInfo::detect(),
        };

        let store = CheckpointStore::new();

        // Load previous results for comparison (same bench name only)
        let previous: Vec<_> = store
            .list_recent(50)
            .unwrap_or_default()
            .into_iter()
            .filter(|c| c.bench_name == config.bench_name)
            .take(5)
            .collect();

        let snapshot = summary.to_snapshot();

        // Print comparison if we have previous results
        print_comparison(&snapshot, &server_info, &previous);

        let checkpoint = LoadTestCheckpoint::new(
            config.bench_name.clone(),
            server_info,
            environment,
            config,
            snapshot,
            summary.errors.into_iter().take(10).collect(),
        );

        match store.save(&checkpoint) {
            Ok(path) => {
                println!();
                println!("Results saved to: {}", path.display());
            }
            Err(e) => {
                eprintln!("Failed to save results: {}", e);
            }
        }
    }

    Ok(())
}
