//! Agent definitions shared between sessions.
//!
//! Decision: every session carries its agent's definition, and most
//! sessions run one of a few agents. Identical definitions share one
//! allocation (interned by their JSON), so a shard holds each agent once in
//! memory and once per snapshot, however many sessions run it.

use std::collections::HashMap;
use std::sync::{Arc, Weak};

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;

static TABLE: parking_lot::Mutex<Option<HashMap<String, Weak<Value>>>> =
    parking_lot::Mutex::new(None);

/// A stable key for `definition`: FNV-1a of its JSON (maps serialize
/// in key order, so equal definitions have equal JSON).
pub fn key(definition: &Value) -> String {
    let json = serde_json::to_vec(definition).unwrap_or_default();
    let hash = json.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, b| {
        (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3)
    });
    format!("{hash:016x}-{}", json.len())
}

/// The definition interned under `key`, while any session holds it.
pub fn lookup(key: &str) -> Option<Arc<Value>> {
    TABLE.lock().as_ref()?.get(key).and_then(Weak::upgrade)
}

/// The shared copy of `definition`.
pub fn intern(definition: Value) -> Arc<Value> {
    if definition.is_null() {
        return Arc::new(Value::Null);
    }
    let key = key(&definition);
    let mut guard = TABLE.lock();
    let table = guard.get_or_insert_with(HashMap::new);
    if let Some(shared) = table.get(&key).and_then(Weak::upgrade) {
        return shared;
    }
    if table.len() > 1024 {
        table.retain(|_, weak| weak.strong_count() > 0);
    }
    let shared = Arc::new(definition);
    table.insert(key, Arc::downgrade(&shared));
    shared
}

pub fn serialize<S: Serializer>(value: &Arc<Value>, s: S) -> Result<S::Ok, S::Error> {
    value.as_ref().serialize(s)
}

pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Arc<Value>, D::Error> {
    Ok(intern(Value::deserialize(d)?))
}
