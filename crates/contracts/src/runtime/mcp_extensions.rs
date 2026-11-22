//! Everruns MCP extensions: what an injected capability's server can do
//! beyond tools.
//!
//! Decisions:
//! - The host declares the keys under `capabilities.extensions`, like the
//!   MCP Tasks extension, and only to injected capabilities' servers. A host
//!   uses an extension only when the server lists it too. Servers that list
//!   none stay plain MCP servers.
//! - Keys use our reverse-domain prefix (`io.everruns/`), as the spec requires
//!   of custom extensions and as `io.everruns/idempotency-key` already does.
//! - A server lists the extensions it speaks in one resource,
//!   [`EXTENSIONS_RESOURCE_URI`], shaped like `capabilities.extensions`. MCP's
//!   stateless era has no handshake to carry server capabilities, and a
//!   resource works the same in every era.
//! - Facts are a resource the host reads at turn start, not a push, so a
//!   stateless server needs no session state to provide them.
//! - Hooks are requests the host sends, so the server never needs a
//!   server-to-client stream. A server declares which tools it wants to see;
//!   the host calls it only for those.
//! - UI uses MCP's own `structuredContent` and `_meta` on tool results, so it
//!   has no wire types here; the key only says the host renders them.

use crate::runtime::capabilities::{Fact, Volatility};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Server contributes [`Fact`]s through [`FACTS_RESOURCE_URI`].
pub const FACTS_EXTENSION: &str = "io.everruns/facts";
/// Server owns a file tree the host mounts (a session's Memory), read and
/// written through the [`FILES_METHOD_PREFIX`] methods.
pub const FILES_EXTENSION: &str = "io.everruns/files";
/// Server observes or changes tool calls ([`HOOKS_BEFORE_TOOL_METHOD`],
/// [`HOOKS_AFTER_TOOL_METHOD`]).
pub const HOOKS_EXTENSION: &str = "io.everruns/hooks";
/// Host renders `structuredContent` and `_meta` UI on tool results.
pub const UI_EXTENSION: &str = "io.everruns/ui";

/// Every Everruns extension, in the order hosts declare them.
pub const EVERRUNS_EXTENSIONS: [&str; 4] = [
    FACTS_EXTENSION,
    FILES_EXTENSION,
    HOOKS_EXTENSION,
    UI_EXTENSION,
];

/// Resource listing the extensions a server speaks: a JSON object keyed by
/// extension key ([`ServerExtensions`]).
pub const EXTENSIONS_RESOURCE_URI: &str = "everruns://extensions";

/// Resource the host reads at turn start for the server's facts. Its text is a
/// JSON [`FactsDocument`].
pub const FACTS_RESOURCE_URI: &str = "everruns://facts";
pub const FACTS_MIME_TYPE: &str = "application/json";

/// Prefix of the file methods, one per operation: `everruns/files/read`,
/// `everruns/files/write`, ... (see [`FILES_OPS`]). Params carry `path` and
/// the operation's own fields; the result is the operation's value (a
/// `SessionFile`, a `FileInfo` list, a `FileStat`, a bool, or `null` for a
/// file that is not there).
pub const FILES_METHOD_PREFIX: &str = "everruns/files/";
/// The file operations. `write` takes `content`, `encoding` and optionally
/// `if_match` (an [`file_etag`] the file must still have);
/// `write_if_matches` takes `expected_content` and `expected_encoding`
/// instead; `create_if_absent` `content`; `delete` `recursive`. `read` and
/// `stat` of a file add its `etag`.
pub const FILES_OPS: [&str; 8] = [
    "read",
    "write",
    "write_if_matches",
    "create_if_absent",
    "delete",
    "list",
    "stat",
    "mkdir",
];

/// The method of one file operation.
pub fn files_method(op: &str) -> String {
    format!("{FILES_METHOD_PREFIX}{op}")
}

/// The operation a file method names, if it is one.
pub fn files_op(method: &str) -> Option<&'static str> {
    let op = method.strip_prefix(FILES_METHOD_PREFIX)?;
    FILES_OPS.into_iter().find(|known| *known == op)
}

/// A file's entity tag: a hash of its bytes, so the same bytes sent as text
/// or as base64 have one tag.
pub fn file_etag(content: &str, encoding: &str) -> String {
    use sha2::{Digest, Sha256};
    let bytes = crate::runtime::session_file::SessionFile::decode_content(content, encoding)
        .unwrap_or_else(|_| content.as_bytes().to_vec());
    let digest = Sha256::digest(&bytes);
    let hex: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
    format!("\"sha256:{hex}\"")
}

/// A file path as a server takes it: absolute, without `.`, `..` or empty
/// segments, a trailing `/` dropped. THREAT: the server resolves every path
/// inside the session's own tree; refusing escapes here means no service
/// has to get that right on its own.
pub fn checked_file_path(path: &str) -> Result<String, String> {
    if !path.starts_with('/') {
        return Err(format!("{path:?} is not an absolute path"));
    }
    if path.contains('\0') || path.contains('\\') {
        return Err(format!("{path:?} is not a valid path"));
    }
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() {
        return Ok("/".to_string());
    }
    if trimmed[1..]
        .split('/')
        .any(|segment| segment.is_empty() || segment == "." || segment == "..")
    {
        return Err(format!("{path:?} leaves the file tree or is not normal"));
    }
    Ok(trimmed.to_string())
}

pub const HOOKS_BEFORE_TOOL_METHOD: &str = "everruns/hooks/beforeTool";
pub const HOOKS_AFTER_TOOL_METHOD: &str = "everruns/hooks/afterTool";

/// Cap on facts one server may contribute, and on one fact's value. Facts land
/// in every request's prompt.
pub const MAX_FACTS: usize = 32;
pub const MAX_FACT_VALUE_BYTES: usize = 4 * 1024;

/// Body of the [`EXTENSIONS_RESOURCE_URI`] resource: extension key to its
/// settings. Unknown keys are kept and ignored.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ServerExtensions(pub serde_json::Map<String, Value>);

impl ServerExtensions {
    pub fn has(&self, key: &str) -> bool {
        self.0.contains_key(key)
    }

    /// The server's hook subscriptions; empty when it does not speak hooks or
    /// its settings do not parse.
    pub fn hooks(&self) -> HooksDeclaration {
        self.0
            .get(HOOKS_EXTENSION)
            .and_then(|value| serde_json::from_value(value.clone()).ok())
            .unwrap_or_default()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WireVolatility {
    Static,
    Dynamic,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WireFact {
    pub key: String,
    pub value: String,
    pub volatility: WireVolatility,
}

impl From<&Fact> for WireFact {
    fn from(fact: &Fact) -> Self {
        Self {
            key: fact.key.clone(),
            value: fact.value.clone(),
            volatility: match fact.volatility {
                Volatility::Static => WireVolatility::Static,
                Volatility::Dynamic => WireVolatility::Dynamic,
            },
        }
    }
}

impl From<WireFact> for Fact {
    fn from(fact: WireFact) -> Self {
        Fact {
            key: fact.key,
            value: fact.value,
            volatility: match fact.volatility {
                WireVolatility::Static => Volatility::Static,
                WireVolatility::Dynamic => Volatility::Dynamic,
            },
        }
    }
}

/// Body of the [`FACTS_RESOURCE_URI`] resource.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FactsDocument {
    #[serde(default)]
    pub facts: Vec<WireFact>,
}

impl FactsDocument {
    /// Facts the host accepts: at most [`MAX_FACTS`], keys non-empty, values
    /// cut to [`MAX_FACT_VALUE_BYTES`] on a character boundary.
    pub fn into_facts(self) -> Vec<Fact> {
        self.facts
            .into_iter()
            .filter(|fact| !fact.key.trim().is_empty())
            .take(MAX_FACTS)
            .map(|mut fact| {
                if fact.value.len() > MAX_FACT_VALUE_BYTES {
                    let mut end = MAX_FACT_VALUE_BYTES;
                    while !fact.value.is_char_boundary(end) {
                        end -= 1;
                    }
                    fact.value.truncate(end);
                }
                Fact::from(fact)
            })
            .collect()
    }
}

/// What a server lists under its [`HOOKS_EXTENSION`] key: the tool names it
/// wants to see, `*` for all. Names are as the model calls them, so a server
/// may name tools other capabilities provide.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HooksDeclaration {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub before_tool: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub after_tool: Vec<String>,
}

impl HooksDeclaration {
    pub fn wants_before(&self, tool: &str) -> bool {
        names_match(&self.before_tool, tool)
    }

    pub fn wants_after(&self, tool: &str) -> bool {
        names_match(&self.after_tool, tool)
    }

    pub fn is_empty(&self) -> bool {
        self.before_tool.is_empty() && self.after_tool.is_empty()
    }
}

fn names_match(names: &[String], tool: &str) -> bool {
    names.iter().any(|name| name == "*" || name == tool)
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BeforeToolParams {
    pub tool_call_id: String,
    pub tool: String,
    pub arguments: Value,
}

/// A server's answer to [`HOOKS_BEFORE_TOOL_METHOD`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "decision", rename_all = "snake_case")]
pub enum BeforeToolResult {
    /// Run the call, with `arguments` replacing the model's when present.
    Allow {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        arguments: Option<Value>,
    },
    /// Do not run the call. `reason` is what the model reads.
    Deny {
        reason: String,
        #[serde(
            default,
            rename = "userMessage",
            skip_serializing_if = "Option::is_none"
        )]
        user_message: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AfterToolParams {
    pub tool_call_id: String,
    pub tool: String,
    pub arguments: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// A server's answer to [`HOOKS_AFTER_TOOL_METHOD`]. Every field is optional;
/// an empty object leaves the result as it was.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AfterToolResult {
    /// Replaces the result the model reads.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    /// Replaces the error the model reads.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Added to the result as `hook_context`, so the model sees it beside the
    /// tool's own output.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub additional_context: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn file_methods_are_one_per_operation() {
        assert_eq!(files_method("read"), "everruns/files/read");
        assert_eq!(files_op("everruns/files/write"), Some("write"));
        assert_eq!(files_op("everruns/files/chmod"), None);
        assert_eq!(files_op("everruns/files"), None);
        assert_eq!(files_op("everruns/hooks/beforeTool"), None);
    }

    #[test]
    fn a_files_etag_follows_its_bytes_not_their_encoding() {
        let text = file_etag("hi", "text");
        assert_eq!(text, file_etag("aGk=", "base64"));
        assert_ne!(text, file_etag("ho", "text"));
        assert!(text.starts_with("\"sha256:"));
    }

    #[test]
    fn file_paths_stay_inside_the_tree() {
        assert_eq!(
            checked_file_path("/memory/user/a.md").unwrap(),
            "/memory/user/a.md"
        );
        assert_eq!(checked_file_path("/memory/user/").unwrap(), "/memory/user");
        assert_eq!(checked_file_path("/").unwrap(), "/");
        for bad in [
            "memory/a.md",
            "/memory/../etc",
            "/memory/./a",
            "/memory//a",
            "/memory/a/..",
            "/a\0b",
            "/a\\b",
        ] {
            assert!(checked_file_path(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn facts_document_round_trips_and_is_capped() {
        let mut facts: Vec<WireFact> = (0..40)
            .map(|i| WireFact {
                key: format!("k{i}"),
                value: "v".into(),
                volatility: WireVolatility::Dynamic,
            })
            .collect();
        facts[0].key = " ".into();
        facts[1].value = "é".repeat(MAX_FACT_VALUE_BYTES);
        let document = FactsDocument { facts };
        let wire = serde_json::to_value(&document).unwrap();
        assert_eq!(wire["facts"][2]["volatility"], "dynamic");
        let parsed: FactsDocument = serde_json::from_value(wire).unwrap();
        let facts = parsed.into_facts();
        assert_eq!(facts.len(), MAX_FACTS);
        assert_eq!(facts[0].key, "k1");
        assert!(facts[0].value.len() <= MAX_FACT_VALUE_BYTES);
        assert_eq!(facts[1].volatility, Volatility::Dynamic);
    }

    #[test]
    fn hooks_declaration_matches_names_and_star() {
        let declaration: HooksDeclaration =
            serde_json::from_value(json!({"beforeTool": ["write_file"], "afterTool": ["*"]}))
                .unwrap();
        assert!(declaration.wants_before("write_file"));
        assert!(!declaration.wants_before("read_file"));
        assert!(declaration.wants_after("anything"));
        assert!(HooksDeclaration::default().is_empty());
    }

    #[test]
    fn before_tool_result_wire_shape() {
        let allow: BeforeToolResult = serde_json::from_value(json!({"decision": "allow"})).unwrap();
        assert_eq!(allow, BeforeToolResult::Allow { arguments: None });
        let deny: BeforeToolResult = serde_json::from_value(
            json!({"decision": "deny", "reason": "no", "userMessage": "blocked"}),
        )
        .unwrap();
        assert_eq!(
            deny,
            BeforeToolResult::Deny {
                reason: "no".into(),
                user_message: Some("blocked".into())
            }
        );
        assert_eq!(
            serde_json::to_value(BeforeToolParams {
                tool_call_id: "c1".into(),
                tool: "t".into(),
                arguments: json!({})
            })
            .unwrap(),
            json!({"toolCallId": "c1", "tool": "t", "arguments": {}})
        );
    }

    #[test]
    fn server_extensions_expose_hooks() {
        let extensions: ServerExtensions = serde_json::from_value(json!({
            "io.everruns/facts": {},
            "io.everruns/hooks": {"beforeTool": ["*"]},
            "vendor/other": true
        }))
        .unwrap();
        assert!(extensions.has(FACTS_EXTENSION));
        assert!(!extensions.has(UI_EXTENSION));
        assert!(extensions.hooks().wants_before("x"));
        assert!(ServerExtensions::default().hooks().is_empty());
    }

    #[test]
    fn after_tool_result_defaults_to_no_change() {
        let result: AfterToolResult = serde_json::from_value(json!({})).unwrap();
        assert_eq!(result, AfterToolResult::default());
    }
}
