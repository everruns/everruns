//! Shared trigger filtering, change detection and template rendering.
use everruns_contracts::runtime::triggers::TriggerEventFilter;
use regex::Regex;
use serde_json::Value;
use std::sync::LazyLock;
#[expect(
    clippy::expect_used,
    reason = "constant template expression; invalid syntax is a programming error"
)]
static TEMPLATE_EXPR_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\{\{\s*([a-zA-Z0-9_.-]+)\s*\}\}").expect("template regex is valid")
});

/// Resolve a dotted object/array path in source context.
pub fn template_lookup<'a>(context: &'a Value, path: &str) -> Option<&'a Value> {
    let mut current = context;
    for segment in path.split('.') {
        current = match current {
            Value::Object(map) => map.get(segment)?,
            Value::Array(items) => items.get(segment.parse::<usize>().ok()?)?,
            _ => return None,
        };
    }
    Some(current)
}

fn template_value_to_string(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

/// Render dotted template expressions; missing values become empty text.
pub fn render_message_template(template: &str, context: &Value) -> String {
    TEMPLATE_EXPR_RE
        .replace_all(template, |captures: &regex::Captures<'_>| {
            let path = captures.get(1).map(|m| m.as_str()).unwrap_or_default();
            template_lookup(context, path)
                .map(template_value_to_string)
                .unwrap_or_default()
        })
        .into_owned()
}

/// Explain an absent, first or repeated subject value; None indicates a change.
pub fn change_miss(path: &str, value: Option<&Value>, previous: Option<&Value>) -> Option<String> {
    let Some(value) = value else {
        return Some(format!("{path} is missing"));
    };
    let shown = truncate(&value_label(value), 80);
    match previous {
        None => Some(format!("first {path} for this subject is {shown}")),
        Some(previous) if values_equal(previous, value) => Some(format!("{path} is still {shown}")),
        Some(_) => None,
    }
}

fn values_equal(left: &Value, right: &Value) -> bool {
    values_match(left, right) || values_match(right, left)
}

/// Return why the event misses the filter, or `None` when every condition holds.
pub fn filter_miss(filter: &TriggerEventFilter, context: &Value) -> Option<String> {
    filter.conditions.iter().find_map(|condition| {
        let actual = template_lookup(context, &condition.path);
        let matched = actual.is_some_and(|actual| {
            condition
                .any_of
                .iter()
                .any(|expected| values_match(actual, expected))
        });
        (!matched).then(|| {
            let shown = actual
                .map(|value| truncate(&value_label(value), 80))
                .unwrap_or_else(|| "missing".to_string());
            format!("{} is {shown}", condition.path)
        })
    })
}

fn values_match(actual: &Value, expected: &Value) -> bool {
    if actual == expected {
        return true;
    }
    // A string condition matches the same scalar written differently
    // (`"12"` matches `12`, `"true"` matches `true`), since conditions are
    // often typed in a form.
    match (actual, expected) {
        (Value::Number(number), Value::String(text)) => number.to_string() == text.as_str(),
        (Value::Bool(flag), Value::String(text)) => text.parse::<bool>().ok() == Some(*flag),
        _ => false,
    }
}

fn value_label(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

fn truncate(value: &str, max: usize) -> String {
    if value.len() <= max {
        return value.to_string();
    }
    let mut end = max;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_string()
}
