use std::collections::{HashMap, VecDeque};
use std::sync::{Mutex, OnceLock};

use everruns_provider::ProviderEndpoint;

use crate::driver::{normalize_anthropic_id, split_million_context};

const MAX_ENTRIES: usize = 4_096;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CapabilityState {
    Rejected,
    Confirmed,
}

#[derive(Default)]
struct CapabilityCache {
    entries: HashMap<String, CapabilityState>,
    insertion_order: VecDeque<String>,
}

impl CapabilityCache {
    fn insert(&mut self, key: String, state: CapabilityState) {
        if let Some(existing) = self.entries.get_mut(&key) {
            *existing = state;
            return;
        }
        while self.entries.len() >= MAX_ENTRIES {
            if let Some(oldest) = self.insertion_order.pop_front() {
                self.entries.remove(&oldest);
            }
        }
        self.insertion_order.push_back(key.clone());
        self.entries.insert(key, state);
    }
}

fn cache() -> &'static Mutex<CapabilityCache> {
    static CACHE: OnceLock<Mutex<CapabilityCache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(CapabilityCache::default()))
}

fn key(endpoint: &ProviderEndpoint, model: &str) -> String {
    let endpoint = endpoint.base_url().unwrap_or_default();
    let family = normalize_anthropic_id(split_million_context(model).0).to_ascii_lowercase();
    format!("{endpoint}\n{family}")
}

pub(super) fn is_rejected(endpoint: &ProviderEndpoint, model: &str) -> bool {
    cache().lock().unwrap().entries.get(&key(endpoint, model)) == Some(&CapabilityState::Rejected)
}

/// Record a beta rejection only while support has not already been confirmed.
/// Returns whether the caller may retry with legacy history.
pub(super) fn record_rejection(endpoint: &ProviderEndpoint, model: &str) -> bool {
    let key = key(endpoint, model);
    let mut cache = cache().lock().unwrap();
    if cache.entries.get(&key) == Some(&CapabilityState::Confirmed) {
        return false;
    }
    cache.insert(key, CapabilityState::Rejected);
    true
}

pub(super) fn record_confirmation(endpoint: &ProviderEndpoint, model: &str) {
    cache()
        .lock()
        .unwrap()
        .insert(key(endpoint, model), CapabilityState::Confirmed);
}

pub(super) fn is_rejection_response(status: reqwest::StatusCode, body: &str) -> bool {
    if !matches!(status.as_u16(), 400 | 422) {
        return false;
    }
    let body = body.to_ascii_lowercase();
    body.contains("compact-2026-01-12")
        || body.contains("compact_20260112")
        || body.contains("context_management")
        || body.contains("context management")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejection_classifier_requires_specific_contract_text_and_status() {
        assert!(is_rejection_response(
            reqwest::StatusCode::BAD_REQUEST,
            "unknown beta compact-2026-01-12"
        ));
        assert!(is_rejection_response(
            reqwest::StatusCode::UNPROCESSABLE_ENTITY,
            "context_management.edits is unsupported"
        ));
        assert!(!is_rejection_response(
            reqwest::StatusCode::BAD_REQUEST,
            "invalid max_tokens"
        ));
        assert!(!is_rejection_response(
            reqwest::StatusCode::INTERNAL_SERVER_ERROR,
            "unknown beta compact-2026-01-12"
        ));
    }
}
