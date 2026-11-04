//! Best-effort Messages transport state; never a conversation store. A cache
//! miss sends the caller's complete transcript, including after process loss.
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use web_time::{Duration, Instant};

use everruns_contracts::driver_registry::LlmCallConfig;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

const BETA: &str = "message-threads-2026-08-12";
const OPTION: &str = "anthropic/message_threads";
const MAX_ENTRIES: usize = 1024;
const TTL: Duration = Duration::from_secs(5 * 60);
type Fingerprint = [u8; 32];

struct Entry {
    prefix: Fingerprint,
    messages: usize,
    response_id: String,
    created: Instant,
}

#[derive(Default)]
struct Cache(HashMap<Fingerprint, Entry>);

impl Cache {
    fn claim(&mut self, key: &Fingerprint) -> Option<Entry> {
        // Claim before sending: concurrent turns/forks cannot consume one id twice.
        self.0
            .remove(key)
            .filter(|entry| entry.created.elapsed() < TTL)
    }

    fn insert(&mut self, key: Fingerprint, entry: Entry) {
        self.0.retain(|_, entry| entry.created.elapsed() < TTL);
        if self.0.len() >= MAX_ENTRIES
            && let Some(oldest) = self
                .0
                .iter()
                .min_by_key(|(_, entry)| entry.created)
                .map(|(key, _)| *key)
        {
            self.0.remove(&oldest);
        }
        self.0.insert(key, entry);
    }
}

fn cache() -> &'static Mutex<Cache> {
    static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(Cache::default()))
}

fn fingerprint(value: &Value) -> Fingerprint {
    Sha256::digest(value.to_string().as_bytes()).into()
}

/// Cache markers move between calls, but are not conversation edits. Never strip
/// fields from arbitrary tool arguments/results (they are user data).
fn transcript(messages: &[Value]) -> Vec<Value> {
    let mut messages = messages.to_vec();
    for message in &mut messages {
        if let Some(blocks) = message.get_mut("content").and_then(Value::as_array_mut) {
            for block in blocks {
                if let Some(block) = block.as_object_mut() {
                    block.remove("cache_control");
                }
            }
        }
    }
    messages
}

pub(super) fn add_beta(headers: &mut Vec<(String, String)>) {
    if let Some((_, value)) = headers
        .iter_mut()
        .find(|(name, _)| name.eq_ignore_ascii_case("anthropic-beta"))
    {
        if !value.split(',').any(|beta| beta.trim() == BETA) {
            if !value.is_empty() {
                value.push(',');
            }
            value.push_str(BETA);
        }
    } else {
        headers.push(("anthropic-beta".into(), BETA.into()));
    }
}

pub(super) struct Exchange {
    enabled: bool,
    scope: Value,
    key: Option<Fingerprint>,
    previous: Option<Entry>,
    messages: Vec<Value>,
    pub(super) sent_body: Value,
    failed: bool,
}

impl Exchange {
    pub(super) fn new(config: &LlmCallConfig, context_management: bool) -> Self {
        let session = config
            .metadata
            .get("session_id")
            .filter(|id| !id.is_empty());
        Self {
            enabled: config.driver_options.get(OPTION) == Some(&Value::Bool(true))
                && session.is_some()
                && !context_management
                && config.provider_opaque_context.is_none(),
            scope: json!([config.metadata.get("org_id"), session, config.prompt_cache]),
            key: None,
            previous: None,
            messages: Vec::new(),
            sent_body: Value::Null,
            failed: false,
        }
    }

    pub(super) fn enabled(&self) -> bool {
        self.enabled
    }

    pub(super) fn prepare(
        &mut self,
        mut full: Value,
        url: &str,
        headers: &[(String, String)],
    ) -> Value {
        if self.enabled {
            let mut fixed = full.clone();
            let fields = fixed.as_object_mut().unwrap();
            for free in ["messages", "max_tokens", "stream", "diagnostics"] {
                fields.remove(free);
            }
            let mut headers = headers
                .iter()
                .map(|(name, value)| (name.to_ascii_lowercase(), value))
                .collect::<Vec<_>>();
            headers.sort_unstable();
            // THREAT[TM-LLM-057]: bind state to session, credential/workspace,
            // endpoint and all fixed/create-only settings. Retain only hashes,
            // never credentials or transcript text in the process-wide cache.
            let key = fingerprint(&json!([self.scope, url, headers, fixed]));
            self.messages = transcript(full["messages"].as_array().unwrap());
            if self.key != Some(key) {
                self.key = Some(key);
                self.previous = cache().lock().unwrap().claim(&key).filter(|entry| {
                    self.messages.len() > entry.messages
                        && fingerprint(&json!(&self.messages[..entry.messages])) == entry.prefix
                });
            }
            if let Some(previous) = &self.previous {
                full["messages"] =
                    json!(&full["messages"].as_array().unwrap()[previous.messages..]);
                full["thread"] =
                    json!({"type":"continue", "previous_message_id":previous.response_id});
                full.as_object_mut().unwrap().remove("system");
                full.as_object_mut().unwrap().remove("tools");
            } else {
                full["thread"] = json!({"type":"create"});
            }
        }
        self.sent_body = full.clone();
        full
    }

    /// Replay once on an explicitly identified thread failure, never on an
    /// unrelated 404/model error. A rejected create can fall back unthreaded.
    pub(super) fn recover(&mut self, status: u16, body: &str) -> bool {
        if !self.enabled {
            return false;
        }
        let Ok(error) = serde_json::from_str::<Value>(body) else {
            return false;
        };
        let code = error["error"]["details"]["error_code"].as_str();
        if self.previous.is_some()
            && matches!(
                (status, code),
                (404, Some("thread_not_found"))
                    | (
                        400,
                        Some("thread_already_continued" | "thread_continue_in_flight")
                    )
                    | (400, Some("thread_fingerprint_mismatch"))
            )
        {
            self.previous = None;
            return true;
        }
        if status == 400 && code == Some("thread_unsupported_request") {
            self.enabled = false;
            self.previous = None;
            return true;
        }
        false
    }

    pub(super) fn invalidate(&mut self) {
        self.failed = true;
    }

    /// Called only after message_stop (or a validated complete JSON response).
    pub(super) fn complete(&mut self, id: Option<&str>, stop: Option<&str>, content: &[Value]) {
        if !self.enabled
            || self.failed
            || !matches!(stop, Some("end_turn" | "tool_use" | "stop_sequence"))
        {
            return;
        }
        let (Some(key), Some(id)) = (self.key, id.filter(|id| !id.is_empty() && id.len() <= 1024))
        else {
            return;
        };
        let mut messages = self.messages.clone();
        messages.push(json!({"role":"assistant", "content":content}));
        let messages = transcript(&messages);
        cache().lock().unwrap().insert(
            key,
            Entry {
                prefix: fingerprint(&json!(messages)),
                messages: messages.len(),
                response_id: id.to_owned(),
                created: Instant::now(),
            },
        );
        self.failed = true; // A duplicate terminal event cannot publish twice.
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(created: Instant) -> Entry {
        Entry {
            prefix: [1; 32],
            messages: 2,
            response_id: "msg_test".into(),
            created,
        }
    }

    #[test]
    fn cache_is_bounded_expires_and_claims_once() {
        let mut cache = Cache::default();
        let now = Instant::now();
        cache.insert([0; 32], entry(now - TTL));
        assert!(cache.claim(&[0; 32]).is_none());
        for i in 0..MAX_ENTRIES + 1 {
            cache.insert(fingerprint(&json!(i)), entry(now));
        }
        assert_eq!(cache.0.len(), MAX_ENTRIES);
        let key = *cache.0.keys().next().unwrap();
        assert!(cache.claim(&key).is_some());
        assert!(cache.claim(&key).is_none());
    }

    #[test]
    fn normalization_preserves_tool_data_and_only_ignores_cache_markers() {
        let messages = vec![json!({"role":"assistant", "content":[{
            "type":"tool_use", "input":{"cache_control":"user data"},
            "cache_control":{"type":"ephemeral"}
        }]})];
        let normalized = transcript(&messages);
        assert!(normalized[0]["content"][0].get("cache_control").is_none());
        assert_eq!(
            normalized[0]["content"][0]["input"]["cache_control"],
            "user data"
        );
        let mut changed = normalized.clone();
        changed[0]["content"][0]["input"]["cache_control"] = json!("edited");
        assert_ne!(
            fingerprint(&json!(changed)),
            fingerprint(&json!(normalized))
        );
    }

    #[test]
    fn compaction_and_missing_scope_disable_threads() {
        let mut config = LlmCallConfig::new("claude-haiku-4-5");
        config.driver_options.insert(OPTION.into(), json!(true));
        assert!(!Exchange::new(&config, false).enabled());
        config
            .metadata
            .insert("session_id".into(), "session".into());
        assert!(Exchange::new(&config, false).enabled());
        assert!(!Exchange::new(&config, true).enabled());
        config.provider_opaque_context = Some(
            everruns_contracts::ProviderOpaqueContext::AnthropicMessagesPrefix {
                messages_json: "[]".into(),
            },
        );
        assert!(!Exchange::new(&config, false).enabled());
    }

    #[test]
    fn beta_header_preserves_caller_features_without_duplicates() {
        let mut headers = vec![("Anthropic-Beta".into(), "other-beta".into())];
        add_beta(&mut headers);
        add_beta(&mut headers);
        assert_eq!(
            headers,
            vec![("Anthropic-Beta".into(), format!("other-beta,{BETA}"))]
        );
    }
}
