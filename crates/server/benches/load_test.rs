//! Everruns Load Test
//!
//! Massive load testing for Everruns with llmsim. Supports:
//! - Parallel session execution
//! - Thousands of messages per session
//! - Realistic LLM timing simulation
//! - Chaos scenarios (timeouts, errors, rate limits)
//!
//! Usage:
//!   cargo run --release -p everruns-control-plane --example load_test
//!   # Or via just:
//!   just load-test
//!
//! Configuration via environment:
//!   API_URL=http://localhost:9000   # API endpoint
//!   SESSIONS=100                    # Number of parallel sessions
//!   MESSAGES_PER_SESSION=50         # Messages per session
//!   MODEL_ID=llmsim                 # Model to use (llmsim, llmsim-ttft-500, etc.)

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use futures::stream::{self, StreamExt};
use parking_lot::Mutex;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tokio::sync::Semaphore;

// ============================================================================
// Configuration
// ============================================================================

#[derive(Clone)]
struct LoadTestConfig {
    api_url: String,
    org: String,
    sessions: usize,
    messages_per_session: usize,
    model_id: String,
    max_concurrent_sessions: usize,
    timeout_secs: u64,
}

impl Default for LoadTestConfig {
    fn default() -> Self {
        Self {
            api_url: std::env::var("API_URL")
                .unwrap_or_else(|_| "http://localhost:9000".to_string()),
            org: std::env::var("ORG").unwrap_or_else(|_| "default".to_string()),
            sessions: std::env::var("SESSIONS")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(100),
            messages_per_session: std::env::var("MESSAGES_PER_SESSION")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(50),
            model_id: std::env::var("MODEL_ID").unwrap_or_else(|_| "llmsim".to_string()),
            max_concurrent_sessions: std::env::var("MAX_CONCURRENT")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(50),
            timeout_secs: std::env::var("TIMEOUT_SECS")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(300),
        }
    }
}

// ============================================================================
// API Types
// ============================================================================

#[derive(Debug, Serialize)]
struct CreateAgentRequest {
    name: String,
    system_prompt: String,
    default_model_id: String,
}

#[derive(Debug, Deserialize)]
struct Agent {
    id: String,
}

#[derive(Debug, Serialize)]
struct CreateSessionRequest {
    title: Option<String>,
    model_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Session {
    id: String,
}

#[derive(Debug, Serialize)]
struct CreateMessageRequest {
    message: InputMessage,
}

#[derive(Debug, Serialize)]
struct InputMessage {
    content: Vec<ContentPart>,
}

#[derive(Debug, Serialize)]
struct ContentPart {
    #[serde(rename = "type")]
    content_type: String,
    text: String,
}

#[derive(Debug, Deserialize)]
struct Message {
    #[allow(dead_code)]
    id: String,
}

#[derive(Debug, Deserialize)]
struct Event {
    #[serde(rename = "type")]
    event_type: String,
}

#[derive(Debug, Deserialize)]
struct EventsResponse {
    items: Vec<Event>,
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
            // Cap error collection
            errors.push(error);
        }
    }

    fn summary(&self) -> MetricsSummary {
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

        MetricsSummary {
            sessions_created: self.sessions_created.load(Ordering::Relaxed),
            sessions_completed: self.sessions_completed.load(Ordering::Relaxed),
            sessions_failed: self.sessions_failed.load(Ordering::Relaxed),
            messages_sent: self.messages_sent.load(Ordering::Relaxed),
            messages_completed: self.messages_completed.load(Ordering::Relaxed),
            messages_failed: self.messages_failed.load(Ordering::Relaxed),
            latency_p50: p50,
            latency_p95: p95,
            latency_p99: p99,
            latency_avg: avg,
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
    errors: Vec<String>,
}

// ============================================================================
// Load Test Runner
// ============================================================================

struct LoadTestRunner {
    config: LoadTestConfig,
    client: Client,
    metrics: Arc<Metrics>,
}

impl LoadTestRunner {
    fn new(config: LoadTestConfig) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(config.timeout_secs))
            .pool_max_idle_per_host(100)
            .build()
            .expect("Failed to create HTTP client");

        Self {
            config,
            client,
            metrics: Arc::new(Metrics::default()),
        }
    }

    async fn create_load_test_agent(&self) -> anyhow::Result<String> {
        let url = format!("{}/v1/orgs/{}/agents", self.config.api_url, self.config.org);

        let req = CreateAgentRequest {
            name: format!("Load Test Agent {}", chrono::Utc::now().timestamp()),
            system_prompt: "You are a helpful assistant for load testing. Respond concisely."
                .to_string(),
            default_model_id: self.config.model_id.clone(),
        };

        let resp = self
            .client
            .post(&url)
            .json(&req)
            .send()
            .await?
            .error_for_status()?
            .json::<Agent>()
            .await?;

        Ok(resp.id)
    }

    async fn create_session(&self, agent_id: &str, session_num: usize) -> anyhow::Result<String> {
        let url = format!(
            "{}/v1/orgs/{}/agents/{}/sessions",
            self.config.api_url, self.config.org, agent_id
        );

        let req = CreateSessionRequest {
            title: Some(format!("Load Test Session {}", session_num)),
            model_id: Some(self.config.model_id.clone()),
        };

        let resp = self
            .client
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

    async fn send_message(
        &self,
        agent_id: &str,
        session_id: &str,
        message_num: usize,
    ) -> anyhow::Result<Duration> {
        let url = format!(
            "{}/v1/orgs/{}/agents/{}/sessions/{}/messages",
            self.config.api_url, self.config.org, agent_id, session_id
        );

        let start = Instant::now();

        let req = CreateMessageRequest {
            message: InputMessage {
                content: vec![ContentPart {
                    content_type: "text".to_string(),
                    text: format!(
                        "Load test message {} of {}. Please respond briefly.",
                        message_num + 1,
                        self.config.messages_per_session
                    ),
                }],
            },
        };

        self.metrics.messages_sent.fetch_add(1, Ordering::Relaxed);

        let _msg = self
            .client
            .post(&url)
            .json(&req)
            .send()
            .await?
            .error_for_status()?
            .json::<Message>()
            .await?;

        // Wait for turn completion by polling events
        self.wait_for_turn_completion(agent_id, session_id).await?;

        let duration = start.elapsed();
        self.metrics.record_latency(duration);
        self.metrics
            .messages_completed
            .fetch_add(1, Ordering::Relaxed);

        Ok(duration)
    }

    async fn wait_for_turn_completion(
        &self,
        agent_id: &str,
        session_id: &str,
    ) -> anyhow::Result<()> {
        let base_url = format!(
            "{}/v1/orgs/{}/agents/{}/sessions/{}/events",
            self.config.api_url, self.config.org, agent_id, session_id
        );

        let timeout = Duration::from_secs(60);
        let start = Instant::now();
        let mut last_event_count = 0;

        loop {
            if start.elapsed() > timeout {
                return Err(anyhow::anyhow!("Timeout waiting for turn completion"));
            }

            let url = format!("{}?limit=100&offset={}", base_url, last_event_count);
            let resp = self
                .client
                .get(&url)
                .send()
                .await?
                .error_for_status()?
                .json::<EventsResponse>()
                .await?;

            last_event_count += resp.items.len();

            // Check for session.idled or session.completed
            let completed = resp.items.iter().any(|e| {
                e.event_type == "session.idled"
                    || e.event_type == "session.completed"
                    || e.event_type == "session.failed"
            });

            if completed {
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

        // Send messages sequentially within session
        for msg_num in 0..self.config.messages_per_session {
            match self.send_message(agent_id, &session_id, msg_num).await {
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

    async fn run(&self) -> anyhow::Result<MetricsSummary> {
        println!("═══════════════════════════════════════════════════════════");
        println!("              Everruns Load Test");
        println!("═══════════════════════════════════════════════════════════");
        println!();
        println!("Configuration:");
        println!("  API URL:              {}", self.config.api_url);
        println!("  Organization:         {}", self.config.org);
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
        println!();

        // Create load test agent
        println!("🚀 Creating load test agent...");
        let agent_id = self.create_load_test_agent().await?;
        println!("   Agent ID: {}", agent_id);
        println!();

        // Semaphore for concurrent session limit
        let semaphore = Arc::new(Semaphore::new(self.config.max_concurrent_sessions));

        let start_time = Instant::now();
        let total_messages = self.config.sessions * self.config.messages_per_session;

        println!(
            "🔥 Starting load test with {} sessions ({} total messages)...",
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
        let summary = self.metrics.summary();

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
        println!(
            "  Throughput: {:.1} msg/sec",
            summary.messages_completed as f64 / total_duration.as_secs_f64()
        );
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

        Ok(summary)
    }
}

impl Clone for LoadTestRunner {
    fn clone(&self) -> Self {
        Self {
            config: self.config.clone(),
            client: self.client.clone(),
            metrics: self.metrics.clone(),
        }
    }
}

// ============================================================================
// Main
// ============================================================================

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Parse CLI args
    let args: Vec<String> = std::env::args().collect();

    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("Everruns Load Test");
        println!();
        println!("Usage: load_test [OPTIONS]");
        println!();
        println!("Environment Variables:");
        println!("  API_URL              API endpoint (default: http://localhost:9000)");
        println!("  ORG                  Organization (default: default)");
        println!("  SESSIONS             Number of parallel sessions (default: 100)");
        println!("  MESSAGES_PER_SESSION Messages per session (default: 50)");
        println!("  MODEL_ID             Model ID (default: llmsim)");
        println!("  MAX_CONCURRENT       Max concurrent sessions (default: 50)");
        println!("  TIMEOUT_SECS         Request timeout in seconds (default: 300)");
        println!();
        println!("Model ID options for different scenarios:");
        println!("  llmsim               - Fast responses (no latency)");
        println!("  llmsim-ttft-100      - 100ms delay before first token");
        println!("  llmsim-ttft-500      - 500ms delay (realistic)");
        println!("  llmsim-ttft-2000     - 2s delay (slow model simulation)");
        println!();
        println!("Examples:");
        println!("  # Basic load test (100 sessions, 50 messages each = 5000 total)");
        println!("  just load-test");
        println!();
        println!("  # High volume test");
        println!("  SESSIONS=500 MESSAGES_PER_SESSION=100 just load-test");
        println!();
        println!("  # With realistic LLM delays");
        println!("  MODEL_ID=llmsim-ttft-500 just load-test");
        println!();
        println!("  # Stress test with many concurrent sessions");
        println!("  SESSIONS=1000 MAX_CONCURRENT=200 just load-test");
        println!();
        return Ok(());
    }

    let config = LoadTestConfig::default();
    let runner = LoadTestRunner::new(config);

    runner.run().await?;

    Ok(())
}
