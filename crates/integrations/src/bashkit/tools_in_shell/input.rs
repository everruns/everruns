//! Reading a tool's input from what a script typed.
//!
//! Decision: a tool already takes one JSON object checked against its schema,
//! so the command takes that object rather than a flag grammar per tool. The
//! relaxed forms (`key=value`, `--key value`) are a convenience over the same
//! object, not a second contract: they merge over the JSON, and the tool's own
//! schema check is the only validation.

use serde_json::{Map, Value};

/// What the arguments after the command path asked for.
#[derive(Debug, PartialEq)]
pub(crate) enum Request {
    Help,
    Call(Value),
}

/// Build the input object from argv and stdin.
///
/// Order, lowest to highest precedence: stdin JSON, a JSON argument, then
/// `key=value` pairs and `--key value` flags. A value is read as JSON when it
/// parses and the schema does not say the key is a string; otherwise it is a
/// string. Key spellings resolve against the schema's properties, so
/// `--repo-name` and `repo_name=` both reach `repo_name`.
pub(crate) fn parse(
    args: &[String],
    stdin: Option<&str>,
    schema: &Value,
) -> Result<Request, String> {
    if args.iter().any(|a| a == "--help" || a == "-h") {
        return Ok(Request::Help);
    }
    let mut object = Map::new();
    let mut read_stdin = stdin.is_some_and(|s| !s.trim().is_empty());
    if args.iter().any(|a| a == "-") {
        if !read_stdin {
            return Err("`-` reads the input from stdin, but stdin is empty".to_string());
        }
        read_stdin = true;
    }
    if read_stdin && let Some(text) = stdin {
        merge(&mut object, parse_object(text.trim(), "stdin")?);
    }

    let mut index = 0;
    while index < args.len() {
        let arg = &args[index];
        index += 1;
        if arg == "-" {
            continue;
        }
        if arg.trim_start().starts_with('{') {
            merge(&mut object, parse_object(arg, "the JSON argument")?);
            continue;
        }
        if let Some(flag) = arg.strip_prefix("--") {
            if flag.is_empty() {
                continue;
            }
            let (key, value) = match flag.split_once('=') {
                Some((key, value)) => (key, Some(value.to_string())),
                None => {
                    // `--flag` alone is `true`; `--key value` takes the next word
                    // unless that word is itself a flag.
                    let next = args.get(index).filter(|n| !n.starts_with("--"));
                    match next {
                        Some(value) => {
                            index += 1;
                            (flag, Some(value.clone()))
                        }
                        None => (flag, None),
                    }
                }
            };
            let key = resolve_key(schema, key);
            let value = match value {
                Some(raw) => typed_value(schema, &key, &raw),
                None => Value::Bool(true),
            };
            object.insert(key, value);
            continue;
        }
        if let Some((key, raw)) = arg.split_once('=')
            && !key.is_empty()
        {
            let key = resolve_key(schema, key);
            let value = typed_value(schema, &key, raw);
            object.insert(key, value);
            continue;
        }
        return Err(format!(
            "unexpected argument `{arg}`. Pass the input as one JSON object \
             ('{{\"key\":\"value\"}}'), as key=value, or as --key value"
        ));
    }
    Ok(Request::Call(Value::Object(object)))
}

fn parse_object(text: &str, origin: &str) -> Result<Map<String, Value>, String> {
    match serde_json::from_str::<Value>(text) {
        Ok(Value::Object(map)) => Ok(map),
        Ok(_) => Err(format!("{origin} must be a JSON object")),
        Err(error) => Err(format!("{origin} is not valid JSON: {error}")),
    }
}

fn merge(into: &mut Map<String, Value>, from: Map<String, Value>) {
    for (key, value) in from {
        into.insert(key, value);
    }
}

/// The schema property a typed key means: exact, then with hyphens read as
/// underscores, then ignoring case and separators. Unknown keys pass through
/// with hyphens as underscores, and the schema check reports them.
fn resolve_key(schema: &Value, key: &str) -> String {
    let Some(properties) = schema.get("properties").and_then(Value::as_object) else {
        return key.replace('-', "_");
    };
    if properties.contains_key(key) {
        return key.to_string();
    }
    let snake = key.replace('-', "_");
    if properties.contains_key(&snake) {
        return snake;
    }
    let loose = |s: &str| s.to_ascii_lowercase().replace(['-', '_'], "");
    let wanted = loose(key);
    properties
        .keys()
        .find(|name| loose(name) == wanted)
        .cloned()
        .unwrap_or(snake)
}

/// `42` is a number unless the schema says the key is a string.
fn typed_value(schema: &Value, key: &str, raw: &str) -> Value {
    let wants_string = schema
        .get("properties")
        .and_then(|p| p.get(key))
        .and_then(|p| p.get("type"))
        .is_some_and(|t| match t {
            Value::String(t) => t == "string",
            Value::Array(types) => {
                types.iter().any(|t| t == "string")
                    && !types.iter().any(|t| t != "string" && t != "null")
            }
            _ => false,
        });
    if wants_string {
        return Value::String(raw.to_string());
    }
    serde_json::from_str(raw).unwrap_or_else(|_| Value::String(raw.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    fn schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                "repo": {"type": "string"},
                "number": {"type": "integer"},
                "issue_title": {"type": "string"},
                "draft": {"type": "boolean"},
                "labels": {"type": "array"},
                "pageSize": {"type": "integer"}
            }
        })
    }

    fn call(list: &[&str], stdin: Option<&str>) -> Value {
        match parse(&args(list), stdin, &schema()).unwrap() {
            Request::Call(value) => value,
            Request::Help => panic!("expected a call"),
        }
    }

    #[test]
    fn json_argument_is_the_input() {
        assert_eq!(
            call(&[r#"{"repo":"a/b","number":42}"#], None),
            json!({"repo": "a/b", "number": 42})
        );
    }

    #[test]
    fn stdin_json_is_the_input() {
        assert_eq!(call(&[], Some("{\"number\": 7}\n")), json!({"number": 7}));
        assert_eq!(call(&["-"], Some("{\"number\": 7}")), json!({"number": 7}));
    }

    #[test]
    fn pairs_and_flags_merge_over_json() {
        assert_eq!(
            call(
                &[r#"{"repo":"a/b","number":1}"#, "number=2", "--draft"],
                Some(r#"{"labels":["ci"]}"#)
            ),
            json!({"repo": "a/b", "number": 2, "draft": true, "labels": ["ci"]})
        );
    }

    #[test]
    fn schema_string_keeps_digits_as_text() {
        assert_eq!(call(&["repo=42"], None), json!({"repo": "42"}));
        assert_eq!(call(&["number=42"], None), json!({"number": 42}));
        assert_eq!(call(&["labels=[\"a\"]"], None), json!({"labels": ["a"]}));
    }

    #[test]
    fn kebab_and_camel_keys_resolve_to_schema_names() {
        assert_eq!(
            call(&["--issue-title", "Flaky test", "--page-size=5"], None),
            json!({"issue_title": "Flaky test", "pageSize": 5})
        );
    }

    #[test]
    fn a_flag_followed_by_a_flag_is_true() {
        assert_eq!(
            call(&["--draft", "--number", "3"], None),
            json!({"draft": true, "number": 3})
        );
    }

    #[test]
    fn help_wins() {
        assert_eq!(
            parse(&args(&["number=1", "--help"]), None, &schema()).unwrap(),
            Request::Help
        );
    }

    #[test]
    fn bad_input_is_reported() {
        assert!(parse(&args(&["{not json"]), None, &schema()).is_err());
        assert!(parse(&args(&["[1,2]"]), None, &schema()).is_err());
        assert!(parse(&args(&["stray"]), None, &schema()).is_err());
        assert!(parse(&args(&["-"]), None, &schema()).is_err());
        assert!(parse(&args(&[]), Some("not json"), &schema()).is_err());
    }

    #[test]
    fn no_input_is_an_empty_object() {
        assert_eq!(call(&[], None), json!({}));
        assert_eq!(call(&[], Some("  \n")), json!({}));
    }
}
