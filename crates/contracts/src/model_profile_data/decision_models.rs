//! Exact System One wire aliases shared by catalog sync and transport.

/// Translate supported Jev aliases without interpreting arbitrary model names.
/// Sources: https://openrouter.ai/docs/guides/community/jev and
/// https://docs.typesafe.ai/models. Provider selection remains explicit.
pub fn openrouter_decision_model_id(model: &str) -> &str {
    match model {
        "jev-1.14.0" => "typesafe/jev-1.14",
        "jev-1.13.0" => "typesafe/jev-1.13",
        "jev-latest" | "typesafe/jev-latest" => "~typesafe/jev-latest",
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model_profile_data::{
        ServiceKind, get_model_profile_key, profile_entries_for_provider,
    };

    #[test]
    fn synced_decision_wire_ids_retain_their_profile_identity() {
        let entries = profile_entries_for_provider("openrouter");
        let decisions: Vec<_> = entries
            .iter()
            .filter(|entry| entry.service == ServiceKind::Decisions)
            .collect();
        let wire_ids: Vec<_> = decisions
            .iter()
            .map(|entry| openrouter_decision_model_id(&entry.model_id))
            .collect();
        assert!(wire_ids.contains(&"typesafe/jev-1.14"));
        assert!(wire_ids.contains(&"typesafe/jev-1.13"));
        assert!(wire_ids.contains(&"~typesafe/jev-latest"));
        for entry in decisions {
            let wire = openrouter_decision_model_id(&entry.model_id);
            assert_eq!(
                get_model_profile_key("openrouter", wire).as_deref(),
                Some(entry.key.as_str())
            );
        }
        for opaque in [
            "jev-preview",
            "other/jev-1.14.0",
            "typesafe/jev-1.14-20261014",
        ] {
            assert_eq!(openrouter_decision_model_id(opaque), opaque);
        }
    }
}
