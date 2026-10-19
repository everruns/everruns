//! Transport-neutral binding enforcement and known-value sanitation.
use super::session_services::{SecretAuth, SecretBinding};
use serde::de::{self, Deserialize, Deserializer, MapAccess, SeqAccess, Visitor};
use serde_json::{Value, json};
use std::collections::BTreeMap;

pub fn validate_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && name
            .bytes()
            .enumerate()
            .all(|(i, b)| b == b'_' || b.is_ascii_alphabetic() || (i > 0 && b.is_ascii_digit()))
}

fn unambiguous_path(url: &url::Url) -> bool {
    let path = url.path().to_ascii_lowercase();
    !["%2f", "%5c", "%2e"].iter().any(|part| path.contains(part)) && !url.as_str().contains('\\')
}

pub fn binding_matches(binding: &SecretBinding, destination: &str) -> bool {
    let (Ok(bound), Ok(target)) = (url::Url::parse(&binding.url), url::Url::parse(destination))
    else {
        return false;
    };
    if binding.validate().is_err()
        || !unambiguous_path(&bound)
        || !unambiguous_path(&target)
        || target.scheme() != "https"
        || target.origin() != bound.origin()
        || !target.username().is_empty()
        || target.password().is_some()
        || target.fragment().is_some()
    {
        return false;
    }
    let prefix = bound.path().trim_end_matches('/');
    target.path() == prefix
        || target
            .path()
            .strip_prefix(prefix)
            .is_some_and(|rest| rest.starts_with('/'))
}

pub fn same_origin(a: &str, b: &str) -> bool {
    match (url::Url::parse(a), url::Url::parse(b)) {
        (Ok(a), Ok(b)) => a.scheme() == "https" && a.origin() == b.origin(),
        _ => false,
    }
}

pub fn expected_header(auth: &SecretAuth) -> Option<(&str, &str)> {
    match auth {
        SecretAuth::Bearer => Some(("authorization", "Bearer ")),
        SecretAuth::Header { name } => Some((name, "")),
        SecretAuth::FormField { .. } => None,
    }
}

#[derive(Clone, Default)]
pub struct CredentialRedactor {
    patterns: Vec<(String, String)>,
    matcher: Option<aho_corasick::AhoCorasick>,
}

impl std::fmt::Debug for CredentialRedactor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CredentialRedactor")
            .field("patterns", &self.patterns.len())
            .finish()
    }
}

impl CredentialRedactor {
    pub fn new<'a>(values: impl IntoIterator<Item = (&'a str, &'a str)>) -> Self {
        let mut patterns = Vec::new();
        for (name, value) in values {
            if value.is_empty() {
                continue;
            }
            // Public reference labels are terminals, including labels already
            // inserted at capture pointers before echo sanitation.
            patterns.push((format!("${name}"), format!("${name}")));
            let escaped = serde_json::to_string(value).unwrap_or_default();
            let escaped = escaped
                .strip_prefix('"')
                .and_then(|v| v.strip_suffix('"'))
                .unwrap_or(value);
            let encoded: String = url::form_urlencoded::byte_serialize(value.as_bytes()).collect();
            for pattern in [
                value.to_string(),
                escaped.to_string(),
                encoded.clone(),
                encoded.replace('+', "%20"),
                lower_percent_escapes(&encoded),
                lower_percent_escapes(&encoded.replace('+', "%20")),
            ] {
                patterns.push((pattern, format!("${name}")));
            }
        }
        patterns.sort_by(|a, b| b.0.len().cmp(&a.0.len()).then_with(|| a.0.cmp(&b.0)));
        patterns.dedup_by(|a, b| a.0 == b.0);
        let matcher = if patterns.is_empty() {
            None
        } else {
            aho_corasick::AhoCorasick::builder()
                .match_kind(aho_corasick::MatchKind::LeftmostLongest)
                .kind(Some(aho_corasick::AhoCorasickKind::ContiguousNFA))
                .build(patterns.iter().map(|(p, _)| p))
                .ok()
        };
        Self { patterns, matcher }
    }

    pub fn ready(&self) -> bool {
        self.patterns.is_empty() || self.matcher.is_some()
    }

    /// Scan the original input once: replacement labels are never reprocessed.
    pub fn text(&self, input: &str) -> String {
        String::from_utf8(self.bytes(input.as_bytes()))
            .unwrap_or_else(|_| "[credential response withheld]".into())
    }

    /// Sanitize a complete bounded response before any bytes enter an
    /// interpreter or model. Decode JSON/form echoes at their semantic boundary.
    pub fn response_body(&self, input: &[u8], content_type: &str) -> Result<Vec<u8>, &'static str> {
        if !self.ready() {
            return Err("Credential sanitizer unavailable");
        }
        let content_type = content_type.to_ascii_lowercase();
        match unique_json(input) {
            Ok(value) => {
                serde_json::to_vec(&self.value(value)).map_err(|_| "Invalid credential JSON")
            }
            Err(_)
                if content_type.contains("json")
                    || std::str::from_utf8(input)
                        .is_ok_and(|s| s.trim_start().starts_with(['{', '['])) =>
            {
                Err("Invalid or ambiguous credential JSON")
            }
            Err(_)
                if content_type
                    .split(';')
                    .next()
                    .is_some_and(|v| v.trim() == "application/x-www-form-urlencoded") =>
            {
                std::str::from_utf8(input).map_err(|_| "Invalid credential form response")?;
                let fields: Vec<_> = url::form_urlencoded::parse(input)
                    .map(|(k, v)| (self.text(&k), self.text(&v)))
                    .collect();
                let mut encoded = url::form_urlencoded::Serializer::new(String::new());
                encoded.extend_pairs(fields);
                Ok(encoded.finish().into_bytes())
            }
            Err(_) => Ok(self.bytes(input)),
        }
    }

    pub fn value(&self, value: Value) -> Value {
        match value {
            Value::String(text) => Value::String(self.text(&text)),
            Value::Array(values) => {
                Value::Array(values.into_iter().map(|v| self.value(v)).collect())
            }
            Value::Object(values) => Value::Object(
                values
                    .into_iter()
                    .map(|(key, value)| (self.text(&key), self.value(value)))
                    .collect(),
            ),
            other => other,
        }
    }

    pub fn bytes(&self, input: &[u8]) -> Vec<u8> {
        let Some(matcher) = &self.matcher else {
            return if self.patterns.is_empty() {
                input.to_vec()
            } else {
                b"[credential response withheld]".to_vec()
            };
        };
        let mut output = Vec::with_capacity(input.len());
        let mut offset = 0;
        for found in matcher.find_iter(input) {
            output.extend_from_slice(&input[offset..found.start()]);
            output.extend_from_slice(self.patterns[found.pattern().as_usize()].1.as_bytes());
            offset = found.end();
        }
        output.extend_from_slice(&input[offset..]);
        output
    }
}

fn lower_percent_escapes(input: &str) -> String {
    let mut bytes = input.as_bytes().to_vec();
    let mut i = 0;
    while i + 2 < bytes.len() {
        if bytes[i] == b'%' {
            bytes[i + 1].make_ascii_lowercase();
            bytes[i + 2].make_ascii_lowercase();
            i += 3;
        } else {
            i += 1;
        }
    }
    String::from_utf8(bytes).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn destinations_match_origins_and_path_segments() {
        let binding = SecretBinding {
            url: "https://api.example/v1".into(),
            auth: SecretAuth::Bearer,
        };
        for url in [
            "https://api.example/v1",
            "https://api.example:443/v1/items?x=1",
        ] {
            assert!(binding_matches(&binding, url), "{url}");
        }
        for url in [
            "http://api.example/v1",
            "https://api.example.evil/v1",
            "https://api.example:444/v1",
            "https://api.example/v10",
            "https://api.example/v1%2fprivate",
            "https://user@api.example/v1",
            "https://api.example/v1#x",
        ] {
            assert!(!binding_matches(&binding, url), "{url}");
        }
    }
    #[test]
    fn echoes_are_sanitized_in_all_supported_encodings_and_short_values() {
        let secret = "q\"& +/\n";
        let redactor = CredentialRedactor::new([("TOKEN", secret), ("SHORT", "z")]);
        let escaped = serde_json::to_string(secret).unwrap();
        let encoded: String = url::form_urlencoded::byte_serialize(secret.as_bytes()).collect();
        let output = redactor.value(serde_json::json!({"body":format!("{secret}|{}|{encoded}|z", &escaped[1..escaped.len()-1]),"headers":{"echo":secret}}));
        assert_eq!(output["body"], "$TOKEN|$TOKEN|$TOKEN|$SHORT");
        assert_eq!(output["headers"]["echo"], "$TOKEN");
    }
}

#[cfg(test)]
mod bounded_redaction_tests {
    use super::*;
    #[test]
    fn inserted_public_reference_labels_are_not_redacted_again() {
        let redactor = CredentialRedactor::new([("ACCESS_TOKEN", "ACCESS")]);
        assert_eq!(
            redactor.text("$ACCESS_TOKEN ACCESS"),
            "$ACCESS_TOKEN $ACCESS_TOKEN"
        );
    }
    #[test]
    fn short_secrets_and_absent_long_patterns_do_not_repeat_searches() {
        let absent = "z".repeat(64 * 1024);
        let redactor = CredentialRedactor::new([("A", "a"), ("LONG", absent.as_str())]);
        assert!(redactor.ready());
        let input = "a".repeat(256 * 1024);
        assert_eq!(redactor.text(&input), "$A".repeat(256 * 1024));
    }
}

struct UniqueJson(Value);
impl<'de> Deserialize<'de> for UniqueJson {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct JsonVisitor;
        impl<'de> Visitor<'de> for JsonVisitor {
            type Value = UniqueJson;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("unambiguous JSON")
            }
            fn visit_bool<E: de::Error>(self, v: bool) -> Result<Self::Value, E> {
                Ok(UniqueJson(json!(v)))
            }
            fn visit_i64<E: de::Error>(self, v: i64) -> Result<Self::Value, E> {
                Ok(UniqueJson(json!(v)))
            }
            fn visit_u64<E: de::Error>(self, v: u64) -> Result<Self::Value, E> {
                Ok(UniqueJson(json!(v)))
            }
            fn visit_f64<E: de::Error>(self, v: f64) -> Result<Self::Value, E> {
                Ok(UniqueJson(json!(v)))
            }
            fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
                Ok(UniqueJson(json!(v)))
            }
            fn visit_string<E: de::Error>(self, v: String) -> Result<Self::Value, E> {
                Ok(UniqueJson(json!(v)))
            }
            fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
                Ok(UniqueJson(Value::Null))
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
                let mut values = Vec::new();
                while let Some(UniqueJson(v)) = seq.next_element()? {
                    values.push(v);
                }
                Ok(UniqueJson(Value::Array(values)))
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                let mut values = BTreeMap::new();
                while let Some((key, UniqueJson(value))) = map.next_entry::<String, UniqueJson>()? {
                    if values.insert(key, value).is_some() {
                        return Err(de::Error::custom("duplicate JSON key"));
                    }
                }
                Ok(UniqueJson(Value::Object(values.into_iter().collect())))
            }
        }
        deserializer.deserialize_any(JsonVisitor)
    }
}
pub fn unique_json(bytes: &[u8]) -> Result<Value, serde_json::Error> {
    serde_json::from_slice::<UniqueJson>(bytes).map(|v| v.0)
}
