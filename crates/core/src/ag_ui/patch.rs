use serde::{Deserialize, Serialize};
use serde_json::Value;

/// An RFC 6902 JSON Patch, as carried by `STATE_DELTA` and `ACTIVITY_DELTA`.
pub type JsonPatch = Vec<JsonPatchOperation>;

/// One JSON Patch operation, tagged by `op`. Paths are JSON Pointers.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "lowercase")]
pub enum JsonPatchOperation {
    Add { path: String, value: Value },
    Remove { path: String },
    Replace { path: String, value: Value },
    Move { from: String, path: String },
    Copy { from: String, path: String },
    Test { path: String, value: Value },
}
