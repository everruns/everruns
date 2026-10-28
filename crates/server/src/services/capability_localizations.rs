//! Fixed display-only overlays for built-in resources. Model instructions and schema values stay intact.
use everruns_contracts::runtime::capability_dto::CapabilityLocalizationInfo;
use everruns_core::CapabilityInfo;
use everruns_core::capabilities::Capability;
use serde_json::{Map, Value};
use std::{collections::BTreeMap, sync::LazyLock};

pub(crate) const ADDITIONAL_LOCALES: [&str; 5] = ["fr", "de", "es", "zh", "en-gb"];
#[expect(
    clippy::expect_used,
    reason = "checked-in catalogs are parsed by localization tests"
)]
static CATALOGS: LazyLock<BTreeMap<String, BTreeMap<String, String>>> = LazyLock::new(|| {
    serde_json::from_str(include_str!("capability_localizations.json"))
        .expect("checked-in display catalogs")
});

pub(crate) fn translated<'a>(locale: &str, source: &'a str) -> &'a str {
    CATALOGS
        .get(locale)
        .and_then(|catalog| catalog.get(source))
        .map(String::as_str)
        .unwrap_or(source)
}

fn translated_field(locale: &str, source: &str) -> Option<String> {
    let value = translated(locale, source);
    (value != source).then(|| value.to_string())
}

/// Only presentation fields enter overlays. Property names, const/enum values, defaults and prompts never do.
fn schema_overlay(schema: &Value, locale: &str, is_overlay: bool) -> Value {
    let mut overlay = Map::new();
    for key in ["title", "description"] {
        if let Some(source) = schema.get(key).and_then(Value::as_str)
            && let Some(value) = translated_field(locale, source)
        {
            overlay.insert(key.into(), value.into());
        }
    }
    if let Some(properties) = schema.get("properties").and_then(Value::as_object) {
        let translated: Map<String, Value> = properties
            .iter()
            .filter_map(|(key, value)| {
                let child = schema_overlay(value, locale, is_overlay);
                (!child.as_object().is_some_and(Map::is_empty)).then(|| (key.clone(), child))
            })
            .collect();
        if !translated.is_empty() {
            overlay.insert("properties".into(), translated.into());
        }
    }
    if let Some(items) = schema.get("items").filter(|items| items.is_object()) {
        let child = schema_overlay(items, locale, is_overlay);
        if !child.as_object().is_some_and(Map::is_empty) {
            overlay.insert("items".into(), child);
        }
    }
    let mut labels = Map::new();
    if is_overlay {
        if let Some(values) = schema.get("enum_labels").and_then(Value::as_object) {
            for (key, value) in values {
                if let Some(source) = value.as_str() {
                    labels.insert(key.clone(), translated(locale, source).into());
                }
            }
        }
    } else if let Some(options) = schema.get("oneOf").and_then(Value::as_array) {
        for option in options {
            if let (Some(value), Some(title)) = (
                option.get("const"),
                option.get("title").and_then(Value::as_str),
            ) {
                let key = value
                    .as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| value.to_string());
                labels.insert(key, translated(locale, title).into());
            }
        }
    }
    if !labels.is_empty() {
        overlay.insert("enum_labels".into(), labels.into());
    }
    overlay.into()
}

pub(super) fn from_core(cap: &dyn Capability) -> CapabilityInfo {
    let mut info = CapabilityInfo::from_core(cap);
    // This path receives the built-in registry, never tenant MCP, skill or declarative rows.
    // A contributed locale wins; fill only fields the capability did not supply.
    for locale in ADDITIONAL_LOCALES {
        let english = info.localizations.get("en");
        let name = translated_field(
            locale,
            english
                .and_then(|e| e.name.as_deref())
                .unwrap_or(&info.name),
        );
        let description = translated_field(
            locale,
            english
                .and_then(|e| e.description.as_deref())
                .unwrap_or(&info.description),
        );
        let config_description = english
            .and_then(|e| e.config_description.as_deref())
            .and_then(|source| translated_field(locale, source));
        let config_overlay = english
            .and_then(|e| e.config_overlay.as_ref())
            .map(|schema| schema_overlay(schema, locale, true))
            .or_else(|| {
                info.config_schema
                    .as_ref()
                    .map(|schema| schema_overlay(schema, locale, false))
            })
            .filter(|overlay| !overlay.as_object().is_some_and(Map::is_empty));
        if name.is_none()
            && description.is_none()
            && config_description.is_none()
            && config_overlay.is_none()
        {
            continue;
        }
        let entry =
            info.localizations
                .entry(locale.into())
                .or_insert_with(|| CapabilityLocalizationInfo {
                    name: None,
                    description: None,
                    config_description: None,
                    config_overlay: None,
                });
        if entry.name.is_none() {
            entry.name = name;
        }
        if entry.description.is_none() {
            entry.description = description;
        }
        if entry.config_description.is_none() {
            entry.config_description = config_description;
        }
        if entry.config_overlay.is_none() {
            entry.config_overlay = config_overlay;
        }
    }
    info
}

#[cfg(test)]
mod tests {
    use super::*;
    struct ContributedCapability;
    impl Capability for ContributedCapability {
        fn id(&self) -> &str {
            "current_time"
        }
        fn name(&self) -> &str {
            "Current Time"
        }
        fn description(&self) -> &str {
            "Customer-owned description"
        }
        fn localizations(&self) -> Vec<everruns_core::capabilities::CapabilityLocalization> {
            vec![everruns_core::capabilities::CapabilityLocalization::text(
                "fr",
                "Contributed name",
                "Contributed description",
            )]
        }
    }

    #[test]
    fn catalog_templates_are_nonempty_and_preserve_arguments() {
        fn tokens(text: &str) -> Vec<String> {
            let mut values: Vec<_> = text
                .split('{')
                .skip(1)
                .filter_map(|part| part.split_once('}').map(|(key, _)| key))
                .filter(|key| {
                    !key.is_empty()
                        && key
                            .chars()
                            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
                })
                .map(str::to_owned)
                .collect();
            values.sort();
            values
        }
        for (locale, catalog) in CATALOGS.iter() {
            for (source, translated) in catalog {
                assert!(!translated.trim().is_empty(), "{locale}: {source}");
                assert_eq!(tokens(source), tokens(translated), "{locale}: {source}");
                for filename in ["AGENTS.md", "SKILL.md", "README.md", "CLAUDE.md"] {
                    if source.contains(filename) {
                        assert!(translated.contains(filename), "{locale}: {source}");
                    }
                }
            }
        }
    }

    #[test]
    fn contributed_copy_wins_and_base_identity_stays_stable() {
        let info = from_core(&ContributedCapability);
        assert_eq!(info.id.as_str(), "current_time");
        assert_eq!(info.name, "Current Time");
        assert_eq!(info.description, "Customer-owned description");
        assert_eq!(
            info.localizations["fr"].name.as_deref(),
            Some("Contributed name")
        );
        assert_eq!(
            info.localizations["fr"].description.as_deref(),
            Some("Contributed description")
        );
        for (locale, name) in [
            ("de", "Aktuelle Uhrzeit"),
            ("es", "Fecha y hora actuales"),
            ("zh", "当前时间"),
        ] {
            assert_eq!(info.localizations[locale].name.as_deref(), Some(name));
        }
    }

    #[test]
    fn schema_overlays_preserve_validation_and_identifiers() {
        let schema = serde_json::json!({"type":"object", "required":["mode"], "properties": {
            "mode": {"title":"Mode", "default":"read", "oneOf":[{"const":"read","title":"Read"},{"const":"write","title":"Write"}]},
            "customer_name": {"title":"Customer-owned text"}
        }});
        let original = schema.clone();
        let overlay = schema_overlay(&schema, "de", false);
        assert_eq!(schema, original);
        assert!(overlay.get("required").is_none());
        assert!(overlay.pointer("/properties/mode/default").is_none());
        assert!(
            overlay
                .pointer("/properties/mode/enum_labels/read")
                .is_some()
        );
        assert!(overlay.pointer("/properties/customer_name").is_none());
    }
}
