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

/// The JSON Patch that turns `old` into `new`.
///
/// Objects diff key by key, recursively: a new key is added, a gone key
/// removed. Arrays diff index by index: common indices recurse, appended items
/// are added at their index, and trailing items are removed last index first,
/// so every path is valid at the point the patch applies it. Anything else
/// that differs is replaced. Equal values give an empty patch.
///
/// ```
/// use everruns_core::ag_ui::{JsonPatchOperation, diff};
/// use serde_json::json;
///
/// let patch = diff(&json!({ "a": [1, 2] }), &json!({ "a": [1, 3, 4] }));
/// assert_eq!(
///     patch,
///     vec![
///         JsonPatchOperation::Replace { path: "/a/1".into(), value: json!(3) },
///         JsonPatchOperation::Add { path: "/a/2".into(), value: json!(4) },
///     ]
/// );
/// ```
pub fn diff(old: &Value, new: &Value) -> JsonPatch {
    let mut patch = JsonPatch::new();
    diff_at(String::new(), old, new, &mut patch);
    patch
}

fn diff_at(path: String, old: &Value, new: &Value, patch: &mut JsonPatch) {
    if old == new {
        return;
    }
    match (old, new) {
        (Value::Object(old), Value::Object(new)) => {
            for key in old.keys().filter(|key| !new.contains_key(*key)) {
                patch.push(JsonPatchOperation::Remove {
                    path: child(&path, key),
                });
            }
            for (key, value) in new {
                match old.get(key) {
                    Some(previous) => diff_at(child(&path, key), previous, value, patch),
                    None => patch.push(JsonPatchOperation::Add {
                        path: child(&path, key),
                        value: value.clone(),
                    }),
                }
            }
        }
        (Value::Array(old), Value::Array(new)) => {
            let common = old.len().min(new.len());
            for (index, (previous, value)) in old.iter().zip(new).enumerate() {
                diff_at(child(&path, &index.to_string()), previous, value, patch);
            }
            for (index, value) in new.iter().enumerate().skip(common) {
                patch.push(JsonPatchOperation::Add {
                    path: child(&path, &index.to_string()),
                    value: value.clone(),
                });
            }
            for index in (common..old.len()).rev() {
                patch.push(JsonPatchOperation::Remove {
                    path: child(&path, &index.to_string()),
                });
            }
        }
        _ => patch.push(JsonPatchOperation::Replace {
            path,
            value: new.clone(),
        }),
    }
}

/// `path` extended by one reference token, escaped per RFC 6901.
fn child(path: &str, token: &str) -> String {
    format!("{path}/{}", token.replace('~', "~0").replace('/', "~1"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Applies the ops `diff` emits (add, remove, replace) per RFC 6902,
    /// panicking where the RFC says the patch fails.
    fn apply(mut doc: Value, patch: &JsonPatch) -> Value {
        for op in patch {
            let (path, value) = match op {
                JsonPatchOperation::Add { path, value }
                | JsonPatchOperation::Replace { path, value } => (path, Some(value.clone())),
                JsonPatchOperation::Remove { path } => (path, None),
                other => panic!("diff emitted {other:?}"),
            };
            if path.is_empty() {
                doc = value.expect("root remove");
                continue;
            }
            let (parent, last) = path.rsplit_once('/').unwrap();
            let last = last.replace("~1", "/").replace("~0", "~");
            match doc.pointer_mut(parent).expect("parent exists") {
                Value::Object(map) => match op {
                    JsonPatchOperation::Remove { .. } => {
                        map.remove(&last).expect("removed key exists");
                    }
                    JsonPatchOperation::Replace { .. } => {
                        assert!(map.contains_key(&last), "replace of a missing key");
                        map.insert(last, value.unwrap());
                    }
                    _ => {
                        map.insert(last, value.unwrap());
                    }
                },
                Value::Array(items) => {
                    let index: usize = last.parse().unwrap();
                    match op {
                        JsonPatchOperation::Add { .. } => {
                            assert!(index <= items.len(), "add past the end");
                            items.insert(index, value.unwrap());
                        }
                        JsonPatchOperation::Remove { .. } => {
                            assert!(index < items.len(), "remove past the end");
                            items.remove(index);
                        }
                        _ => {
                            assert!(index < items.len(), "replace past the end");
                            items[index] = value.unwrap();
                        }
                    }
                }
                other => panic!("cannot apply {op:?} under {other}"),
            }
        }
        doc
    }

    fn round_trip(old: Value, new: Value) -> JsonPatch {
        let patch = diff(&old, &new);
        assert_eq!(apply(old, &patch), new, "patch {patch:?}");
        patch
    }

    #[test]
    fn equal_values_need_no_ops() {
        let value = json!({ "a": [1, { "b": null }] });
        assert!(diff(&value, &value.clone()).is_empty());
    }

    #[test]
    fn object_keys_are_added_removed_and_replaced() {
        let patch = round_trip(
            json!({ "keep": 1, "gone": true, "change": "x" }),
            json!({ "keep": 1, "change": "y", "new": [1] }),
        );
        assert_eq!(patch.len(), 3);
        assert!(patch.contains(&JsonPatchOperation::Remove {
            path: "/gone".into()
        }));
        assert!(patch.contains(&JsonPatchOperation::Replace {
            path: "/change".into(),
            value: json!("y"),
        }));
        assert!(patch.contains(&JsonPatchOperation::Add {
            path: "/new".into(),
            value: json!([1]),
        }));
    }

    #[test]
    fn arrays_grow_and_shrink_from_the_end() {
        let grown = round_trip(json!([1, 2]), json!([1, 5, 3, 4]));
        assert_eq!(grown.len(), 3);
        let shrunk = round_trip(json!([1, 2, 3, 4]), json!([9]));
        assert_eq!(
            shrunk,
            vec![
                JsonPatchOperation::Replace {
                    path: "/0".into(),
                    value: json!(9)
                },
                JsonPatchOperation::Remove { path: "/3".into() },
                JsonPatchOperation::Remove { path: "/2".into() },
                JsonPatchOperation::Remove { path: "/1".into() },
            ]
        );
        round_trip(json!([1]), json!([]));
        round_trip(json!([]), json!([{ "a": 1 }]));
    }

    #[test]
    fn nested_changes_patch_the_leaf() {
        let patch = round_trip(
            json!({ "todos": [{ "content": "a", "status": "pending" }] }),
            json!({ "todos": [{ "content": "a", "status": "completed" }] }),
        );
        assert_eq!(
            patch,
            vec![JsonPatchOperation::Replace {
                path: "/todos/0/status".into(),
                value: json!("completed"),
            }]
        );
    }

    #[test]
    fn type_changes_and_the_root_are_replaced() {
        round_trip(json!({ "a": [1] }), json!({ "a": { "0": 1 } }));
        round_trip(json!([1]), json!({ "a": 1 }));
        let patch = round_trip(json!(1), json!("one"));
        assert_eq!(
            patch,
            vec![JsonPatchOperation::Replace {
                path: String::new(),
                value: json!("one")
            }]
        );
    }

    #[test]
    fn keys_are_escaped_as_json_pointer_tokens() {
        let patch = round_trip(json!({}), json!({ "a/b": 1, "c~d": 2 }));
        let mut paths: Vec<_> = patch
            .iter()
            .map(|op| match op {
                JsonPatchOperation::Add { path, .. } => path.as_str(),
                other => panic!("{other:?}"),
            })
            .collect();
        paths.sort_unstable();
        assert_eq!(paths, ["/a~1b", "/c~0d"]);
        round_trip(
            json!({ "a/b": { "c~d": 1 } }),
            json!({ "a/b": { "c~d": 2 } }),
        );
    }
}
