//! A study-wide, process-safe admission guard. Reservations survive cancellation,
//! provider errors and Mira resume; accounting is never a reason to retry a call.
use everruns_contracts::driver_registry::LlmCompletionMetadata;
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    fs::{self, File, OpenOptions},
    io,
    path::PathBuf,
    time::Instant,
};

pub const CEILING: f64 = 30.0;
const WINDOW: u64 = 1_050_000;

#[derive(Clone)]
pub struct Budget {
    path: PathBuf,
}

#[derive(Default, Serialize, Deserialize)]
struct Ledger {
    version: u32,
    ceiling_usd: f64,
    records: Vec<Value>,
}

fn held(record: &Value) -> io::Result<f64> {
    record["held_usd"]
        .as_f64()
        .filter(|x| x.is_finite() && *x >= 0.0)
        .ok_or_else(|| io::Error::other("invalid budget record"))
}

fn usage_upper_bound(record: &Value) -> io::Result<f64> {
    let (_, output_rate, read_rate, write_rate) = rates(
        record["provider"].as_str().unwrap_or(""),
        record["model"].as_str().unwrap_or(""),
    )?;
    let input = record["input_tokens"]
        .as_u64()
        .ok_or_else(|| io::Error::other("missing accounted input"))?;
    let output = record["output_tokens"]
        .as_u64()
        .ok_or_else(|| io::Error::other("missing accounted output"))?;
    let read = record["cache_read_tokens"].as_u64().unwrap_or(0);
    let write = record["cache_creation_tokens"].as_u64().unwrap_or(0);
    let long = input + read + write >= 272_000;
    Ok(
        ((input + write) as f64 * write_rate + read as f64 * read_rate)
            * if long { 2.0 } else { 1.0 }
            / 1e6
            + output as f64 * output_rate * if long { 1.5 } else { 1.0 } / 1e6,
    )
}

impl Budget {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    fn transaction<T>(&self, f: impl FnOnce(&mut Ledger) -> io::Result<T>) -> io::Result<T> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(self.path.with_extension("lock"))?;
        lock.lock_exclusive()?;
        let mut ledger: Ledger = match fs::read(&self.path) {
            Ok(bytes) => serde_json::from_slice(&bytes)?,
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ledger {
                version: 1,
                ceiling_usd: CEILING,
                records: vec![],
            },
            Err(e) => return Err(e),
        };
        if ledger.version != 1 || ledger.ceiling_usd != CEILING {
            return Err(io::Error::other(
                "incompatible ledger: do not reset an existing study budget",
            ));
        }
        for r in &mut ledger.records {
            held(r)?;
            // Older records used a normal input-price estimate. Tighten their
            // holds in place, preserving estimates and all uncertain attempts.
            // Missing cache-write detail must never replenish the hard budget.
            if r["status"] == "accounted" && r["authoritative_cost"] != true {
                r["held_usd"] = json!(held(r)?.max(usage_upper_bound(r)?));
            }
        }
        let result = f(&mut ledger)?;
        // Commit before sending a request. Lock a separate inode so rename cannot
        // let a second process enter a transaction against a different ledger.
        let temp = self.path.with_extension("tmp");
        fs::write(&temp, serde_json::to_vec_pretty(&ledger)?)?;
        File::open(&temp)?.sync_all()?;
        fs::rename(temp, &self.path)?;
        if let Some(parent) = self.path.parent() {
            File::open(parent)?.sync_all()?;
        }
        Ok(result)
    }

    pub fn records(&self) -> io::Result<Vec<Value>> {
        self.transaction(|l| Ok(l.records.clone()))
    }

    pub fn reserve(
        &self,
        provider: &str,
        model: &str,
        operation: &str,
        input: u64,
        output: u32,
        case: &str,
    ) -> io::Result<usize> {
        rates(provider, model)?;
        let limit = std::env::var("COMPACTION_EVAL_SPEND_LIMIT")
            .map(|value| value.parse::<f64>().map_err(io::Error::other))
            .unwrap_or(Ok(CEILING))?;
        if !limit.is_finite() || !(0.0..=CEILING).contains(&limit) {
            return Err(io::Error::other("invalid study spend limit"));
        }
        let (write_rate, output_rate) = if provider == "openai" {
            (5.0, 15.0)
        } else {
            (2.5, 7.5)
        };
        let reserve = input as f64 * write_rate / 1e6 + f64::from(output) * output_rate / 1e6;
        self.transaction(|l| {
            let total: f64 = l.records.iter().map(held).collect::<io::Result<Vec<_>>>()?.iter().sum();
            if total + reserve > limit || l.records.len() >= 600 || input > WINDOW {
                return Err(io::Error::other("study cost/call/context admission ceiling reached"));
            }
            let id = l.records.len();
            l.records.push(json!({"id":id,"case":case,"provider":provider,"model":model,"operation":operation,
                "reserved_usd":reserve,"held_usd":reserve,"input_upper_bound":input,"output_cap":output,"status":"reserved"}));
            Ok(id)
        })
    }

    pub fn finish(&self, id: usize, meta: &LlmCompletionMetadata, ms: u128) -> io::Result<()> {
        self.transaction(|l| {
            let r = l
                .records
                .get_mut(id)
                .ok_or_else(|| io::Error::other("missing reservation"))?;
            if r["status"] != "reserved" {
                return Ok(());
            }
            let (ir, or, cr, wr) = rates(
                r["provider"].as_str().unwrap_or(""),
                r["model"].as_str().unwrap_or(""),
            )?;
            let input = u64::from(meta.prompt_tokens.unwrap_or(0));
            let read = u64::from(meta.cache_read_tokens.unwrap_or(0));
            let write = u64::from(meta.cache_creation_tokens.unwrap_or(0));
            let long = input + read + write >= 272_000;
            let native = r["operation"] == "native";
            let cost = meta.provider_cost_usd.unwrap_or(
                ((input as f64 * if native { wr } else { ir }
                    + read as f64 * cr
                    + write as f64 * wr)
                    * if long { 2.0 } else { 1.0 }
                    + f64::from(meta.completion_tokens.unwrap_or(0))
                        * or
                        * if long { 1.5 } else { 1.0 })
                    / 1e6,
            );
            let known = meta.provider_cost_usd.is_some()
                || (meta.prompt_tokens.is_some() && meta.completion_tokens.is_some());
            if known && cost.is_finite() && cost >= 0.0 {
                // An unexpected response rate cannot increase the remaining budget.
                r["held_usd"] = json!(cost);
                r["cost_usd"] = json!(cost);
                r["status"] = json!("accounted");
            } else {
                r["status"] = json!("unknown_usage_reservation_retained");
            }
            r["authoritative_cost"] = json!(meta.provider_cost_usd.is_some());
            r["cost_basis"] = json!(if meta.provider_cost_usd.is_some() {
                "provider_reported"
            } else if native {
                "usage_priced_upper_bound"
            } else {
                "usage_priced_estimate"
            });
            r["input_tokens"] = json!(meta.prompt_tokens);
            r["cache_read_tokens"] = json!(meta.cache_read_tokens);
            r["cache_creation_tokens"] = json!(meta.cache_creation_tokens);
            r["output_tokens"] = json!(meta.completion_tokens);
            r["reasoning_tokens"] = json!(meta.reasoning_tokens);
            r["response_model"] = json!(meta.response_model);
            r["duration_ms"] = json!(ms);
            if r["status"] == "accounted" && meta.provider_cost_usd.is_none() {
                r["held_usd"] = json!(cost.max(usage_upper_bound(r)?));
            }
            Ok(())
        })
    }
}

fn rates(provider: &str, model: &str) -> io::Result<(f64, f64, f64, f64)> {
    // Standard, global endpoint, 30m cache only. 2026-11-20 pricing snapshot:
    // https://developers.openai.com/api/docs/pricing and OpenRouter /api/v1/models.
    match (provider, model) {
        ("openai", "gpt-6.2-sol") => Ok((2.0, 10.0, 0.1, 2.5)),
        ("openrouter", "openai/gpt-6.1-terra") => Ok((1.0, 5.0, 0.05, 1.25)),
        _ => Err(io::Error::other("unpriced study destination")),
    }
}

use everruns_contracts::{
    compact::{CompactRequest, CompactResponse},
    driver_registry::{
        ChatDriver, LlmCallConfig, LlmResponse, LlmResponseStream, LlmStreamEvent, Message,
        PromptCacheConfig, PromptCacheRetention,
    },
    error::{AgentLoopError, Result},
    runtime_provider::{Provider, ProviderEndpoint},
};
use futures::StreamExt;

struct Guard {
    inner: Box<dyn ChatDriver>,
    budget: Budget,
    provider: &'static str,
    case: String,
}

impl Guard {
    fn config(&self, config: &LlmCallConfig) -> Result<LlmCallConfig> {
        rates(self.provider, &config.model).map_err(|e| AgentLoopError::config(e.to_string()))?;
        if config.tools.iter().any(|t| t.name() != "query_history") {
            return Err(AgentLoopError::config("study permits only query_history"));
        }
        let mut c = config.clone();
        c.speed = Some("default".into());
        c.max_tokens = Some(c.max_tokens.unwrap_or(4096).min(4096));
        c.prompt_cache = Some(PromptCacheConfig {
            enabled: true,
            retention: PromptCacheRetention::ThirtyMinutes,
            ..Default::default()
        });
        Ok(c)
    }

    fn reserve(&self, messages: &[Message], c: &LlmCallConfig) -> Result<usize> {
        // For plain text UTF-8 bytes bound text tokens; generous envelopes cover
        // role/tool/schema framing. Opaque state reserves the whole model window.
        let opaque = c.provider_opaque_context.is_some()
            || c.previous_response_id.is_some()
            || messages
                .iter()
                .any(|m| !m.reasoning.is_empty() || !m.native_tool_calls.is_empty());
        let input = if opaque {
            WINDOW
        } else {
            (format!("{messages:?}{c:?}").len() as u64 + 8192 + messages.len() as u64 * 512)
                .min(WINDOW)
        };
        self.budget
            .reserve(
                self.provider,
                &c.model,
                if c.metadata
                    .get("purpose")
                    .is_some_and(|p| p == "continuity_summary")
                {
                    "summary"
                } else {
                    "chat"
                },
                input,
                c.max_tokens.unwrap(),
                &self.case,
            )
            .map_err(|e| AgentLoopError::config(e.to_string()))
    }
}

#[async_trait::async_trait]
impl ChatDriver for Guard {
    fn supports_history_summarization(&self) -> bool {
        self.inner.supports_history_summarization()
    }
    fn supports_native_non_streaming(&self) -> bool {
        self.inner.supports_native_non_streaming()
    }
    fn supports_compact(&self) -> bool {
        self.inner.supports_compact()
    }
    fn supports_stateful_responses(&self) -> bool {
        self.inner.supports_stateful_responses()
    }
    fn stream_stall_timeout(&self, c: &LlmCallConfig) -> Option<std::time::Duration> {
        self.inner.stream_stall_timeout(c)
    }
    fn provider_managed_reduction_option(
        &self,
        endpoint: &ProviderEndpoint,
        model: &str,
        budget: usize,
    ) -> Option<(String, Value)> {
        self.inner
            .provider_managed_reduction_option(endpoint, model, budget)
    }
    fn provider_managed_reduction_fallback_reason(
        &self,
        endpoint: &ProviderEndpoint,
        c: &LlmCallConfig,
    ) -> Option<&'static str> {
        self.inner
            .provider_managed_reduction_fallback_reason(endpoint, c)
    }
    fn effective_context_window(&self, m: &str) -> Option<usize> {
        self.inner.effective_context_window(m)
    }
    fn supports_parallel_tool_calls(&self, m: &str) -> bool {
        self.inner.supports_parallel_tool_calls(m)
    }
    fn supports_response_format(&self, m: &str) -> bool {
        self.inner.supports_response_format(m)
    }
    fn validate_provider_opaque_context(
        &self,
        c: &everruns_contracts::driver_registry::ProviderOpaqueContext,
    ) -> bool {
        self.inner.validate_provider_opaque_context(c)
    }
    async fn chat_completion_stream(
        &self,
        e: &ProviderEndpoint,
        m: Vec<Message>,
        c: &LlmCallConfig,
    ) -> Result<LlmResponseStream> {
        let c = self.config(c)?;
        let id = self.reserve(&m, &c)?;
        let started = Instant::now();
        let stream = self.inner.chat_completion_stream(e, m, &c).await?;
        let budget = self.budget.clone();
        Ok(Box::pin(stream.map(move |event| {
            if let Ok(LlmStreamEvent::Done(meta)) = &event {
                budget
                    .finish(id, meta, started.elapsed().as_millis())
                    .map_err(|e| AgentLoopError::config(e.to_string()))?;
            }
            event
        })))
    }
    async fn chat_completion_non_streaming(
        &self,
        e: &ProviderEndpoint,
        m: Vec<Message>,
        c: &LlmCallConfig,
    ) -> Result<LlmResponse> {
        let c = self.config(c)?;
        let id = self.reserve(&m, &c)?;
        let started = Instant::now();
        let r = self.inner.chat_completion_non_streaming(e, m, &c).await?;
        self.budget
            .finish(id, &r.metadata, started.elapsed().as_millis())
            .map_err(|e| AgentLoopError::config(e.to_string()))?;
        Ok(r)
    }
    async fn compact(
        &self,
        e: &ProviderEndpoint,
        r: CompactRequest,
    ) -> Result<Option<CompactResponse>> {
        // Native endpoint has no max-output setting. Reserve its 128K model cap.
        let id = self
            .budget
            .reserve(
                self.provider,
                &r.model,
                "native",
                WINDOW,
                128_000,
                &self.case,
            )
            .map_err(|e| AgentLoopError::config(e.to_string()))?;
        let started = Instant::now();
        let result = self.inner.compact(e, r).await?;
        if let Some(usage) = result.as_ref().and_then(|r| r.usage.as_ref()) {
            let mut meta = LlmCompletionMetadata::default();
            meta.prompt_tokens = usage.input_tokens;
            meta.completion_tokens = usage.output_tokens;
            meta.provider_cost_usd = usage.cost;
            self.budget
                .finish(id, &meta, started.elapsed().as_millis())
                .map_err(|e| AgentLoopError::config(e.to_string()))?;
        }
        Ok(result)
    }
}

pub fn provider(
    name: &'static str,
    budget: Budget,
    case: String,
) -> std::result::Result<Provider, Box<dyn std::error::Error + Send + Sync>> {
    let original: Provider = match name {
        "openai" => everruns::OpenAI::from_env()?.into(),
        "openrouter" => everruns::OpenRouter::from_env()?.into(),
        _ => return Err("unsupported provider".into()),
    };
    let expected = if name == "openai" {
        "https://api.openai.com/v1"
    } else {
        "https://openrouter.ai/api/v1"
    };
    if original
        .endpoint()
        .base_url()
        .is_some_and(|url| url.trim_end_matches('/') != expected)
    {
        return Err("study only prices the global provider endpoints".into());
    }
    Ok(Provider::new(
        name,
        Guard {
            inner: original.into_boxed_driver(),
            budget,
            provider: name,
            case,
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn resume_and_unknown_usage_never_refund_reservations() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("budget.json");
        let b = Budget::new(&path);
        let id = b
            .reserve("openai", "gpt-6.2-sol", "chat", WINDOW, 4096, "a")
            .unwrap();
        b.finish(id, &Default::default(), 0).unwrap();
        let b = Budget::new(path);
        assert!(held(&b.records().unwrap()[0]).unwrap() > 5.0);
        for _ in 0..3 {
            b.reserve("openai", "gpt-6.2-sol", "native", WINDOW, 128000, "b")
                .unwrap();
        }
        assert!(
            b.reserve("openai", "gpt-6.2-sol", "native", WINDOW, 128000, "c")
                .is_err()
        );
    }
    #[test]
    fn malformed_ledger_and_unpriced_models_fail_closed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("budget.json");
        let b = Budget::new(&path);
        assert!(
            b.reserve("openai", "unpriced", "chat", 10, 10, "x")
                .is_err()
        );
        fs::write(path, "broken").unwrap();
        assert!(
            b.reserve("openai", "gpt-6.2-sol", "chat", 10, 10, "x")
                .is_err()
        );
    }
    #[test]
    fn accounting_is_idempotent_and_counts_disjoint_cache_buckets() {
        let dir = tempfile::tempdir().unwrap();
        let b = Budget::new(dir.path().join("b.json"));
        let id = b
            .reserve("openai", "gpt-6.2-sol", "chat", 10000, 1000, "a")
            .unwrap();
        let mut meta = LlmCompletionMetadata::default();
        meta.prompt_tokens = Some(100);
        meta.cache_read_tokens = Some(1000);
        meta.cache_creation_tokens = Some(200);
        meta.completion_tokens = Some(50);
        b.finish(id, &meta, 3).unwrap();
        b.finish(id, &meta, 3).unwrap();
        let record = &b.records().unwrap()[id];
        assert!((record["cost_usd"].as_f64().unwrap() - 0.0013).abs() < 1e-9);
        assert!((held(record).unwrap() - 0.00135).abs() < 1e-9);
    }

    #[test]
    fn concurrent_admission_shares_one_ceiling() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("b.json");
        let workers: Vec<_> = (0..12)
            .map(|_| {
                let path = path.clone();
                std::thread::spawn(move || {
                    Budget::new(path)
                        .reserve(
                            "openai",
                            "gpt-6.2-sol",
                            "native",
                            WINDOW,
                            128000,
                            "parallel",
                        )
                        .is_ok()
                })
            })
            .collect();
        assert_eq!(
            workers
                .into_iter()
                .filter_map(|h| h.join().ok())
                .filter(|ok| *ok)
                .count(),
            4
        );
        assert_eq!(Budget::new(path).records().unwrap().len(), 4);
    }

    #[test]
    fn resuming_old_estimates_cannot_replenish_the_admission_budget() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("b.json");
        fs::write(&path,serde_json::to_vec(&json!({"version":1,"ceiling_usd":30.0,"records":[{
            "provider":"openai","model":"gpt-6.2-sol","status":"accounted","authoritative_cost":false,
            "held_usd":0.0,"cost_usd":0.0,"input_tokens":1050000,"output_tokens":128000
        }]})).unwrap()).unwrap();
        let b = Budget::new(path);
        for _ in 0..3 {
            b.reserve("openai", "gpt-6.2-sol", "native", WINDOW, 128000, "new")
                .unwrap();
        }
        assert!(
            b.reserve("openai", "gpt-6.2-sol", "native", WINDOW, 128000, "new")
                .is_err()
        );
        assert_eq!(
            b.records().unwrap()[0]["cost_usd"],
            0.0,
            "retain original measurement while tightening its hold"
        );
    }
}
