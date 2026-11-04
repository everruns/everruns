//! Backend localization helpers for deterministic platform-authored strings.
//!
//! Decisions:
//! - Resolve supported language tags with exact boundaries; unknown languages use English.
//! - Centralize string catalogs here so backend-authored copy stays externalized
//!   as we add more locales.
//! - Allowed locale and timezone lists are the single source of truth for the
//!   backend. The UI mirrors these lists.

#[path = "localization/activity.rs"]
mod activity;
pub use activity::runtime_text;

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Display-only translations shared by portable agent files and platform resources.
/// Addresses, tool identifiers and execution instructions never use these values.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct EntityLocalization {
    /// Localized display name (1–255 characters when set).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    /// Localized description (1–2000 characters when set).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// Locale-keyed display overlays. Deserialization normalizes tags and bounds input.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(transparent)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct EntityLocalizations(pub BTreeMap<String, EntityLocalization>);

impl EntityLocalizations {
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.0.len() > 64 {
            return Err("localizations: at most 64 locales are allowed".into());
        }
        for (locale, entry) in &self.0 {
            normalize_locale_tag(locale)?;
            for (field, value, limit) in [
                ("display_name", &entry.display_name, 255),
                ("description", &entry.description, 2000),
            ] {
                if let Some(value) = value
                    && (value.trim().is_empty()
                        || value.chars().count() > limit
                        || value.contains('\0'))
                {
                    return Err(format!(
                        "localizations.{locale}.{field}: expected 1–{limit} characters without NUL"
                    ));
                }
            }
        }
        Ok(())
    }

    /// Resolve each field independently: exact tag, language, English, then caller's base.
    pub fn resolve<'a>(
        &'a self,
        locale: &str,
        field: impl Fn(&'a EntityLocalization) -> Option<&'a str>,
    ) -> Option<&'a str> {
        let normalized = normalize_locale_tag(locale).ok()?;
        let language = normalized.split('-').next().unwrap_or("en");
        [normalized.as_str(), language, "en"]
            .into_iter()
            .find_map(|key| self.0.get(key).and_then(&field))
    }
}

impl<'de> Deserialize<'de> for EntityLocalizations {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = BTreeMap::<String, EntityLocalization>::deserialize(deserializer)?;
        let mut entries = BTreeMap::new();
        for (locale, entry) in raw {
            let tag = normalize_locale_tag(&locale).map_err(serde::de::Error::custom)?;
            if entries.insert(tag, entry).is_some() {
                return Err(serde::de::Error::custom(
                    "localizations: duplicate normalized locale",
                ));
            }
        }
        let value = Self(entries);
        value.validate().map_err(serde::de::Error::custom)?;
        Ok(value)
    }
}

/// Normalize language tags for lookup, accepting the underscore spelling used by clients.
pub fn normalize_locale_tag(raw: &str) -> Result<String, String> {
    let tag = raw.trim().replace('_', "-").to_ascii_lowercase();
    let mut parts = tag.split('-');
    let language = parts.next().unwrap_or_default();
    if tag.len() > 64
        || !(2..=8).contains(&language.len())
        || !language.bytes().all(|b| b.is_ascii_alphabetic())
        || parts
            .any(|p| p.is_empty() || p.len() > 8 || !p.bytes().all(|b| b.is_ascii_alphanumeric()))
    {
        return Err(format!("localizations: invalid locale tag {raw:?}"));
    }
    Ok(tag)
}

/// Allowed BCP 47 locale tags. Russia excluded per policy.
// UI mirror: apps/ui/src/lib/locale-data.ts LOCALE_OPTIONS.
pub const ALLOWED_LOCALES: &[&str] = &[
    "af-ZA", "am-ET", "ar-AE", "ar-EG", "ar-SA", "az-AZ", "be-BY", "bg-BG", "bn-BD", "bn-IN",
    "bs-BA", "ca-ES", "cs-CZ", "cy-GB", "da-DK", "de-AT", "de-CH", "de-DE", "el-GR", "en", "en-AU",
    "en-CA", "en-GB", "en-IE", "en-IN", "en-NZ", "en-US", "en-ZA", "es-AR", "es-CL", "es-CO",
    "es-ES", "es-MX", "es-PE", "et-EE", "eu-ES", "fa-IR", "fi-FI", "fil-PH", "fr-BE", "fr-CA",
    "fr-CH", "fr-FR", "ga-IE", "gl-ES", "gu-IN", "he-IL", "hi-IN", "hr-HR", "hu-HU", "hy-AM",
    "id-ID", "is-IS", "it-CH", "it-IT", "ja-JP", "ka-GE", "kk-KZ", "km-KH", "kn-IN", "ko-KR",
    "lo-LA", "lt-LT", "lv-LV", "mk-MK", "ml-IN", "mn-MN", "mr-IN", "ms-MY", "mt-MT", "my-MM",
    "nb-NO", "ne-NP", "nl-BE", "nl-NL", "pa-IN", "pl-PL", "pt-BR", "pt-PT", "ro-RO", "si-LK",
    "sk-SK", "sl-SI", "sq-AL", "sr-RS", "sv-SE", "sw-KE", "ta-IN", "te-IN", "th-TH", "tr-TR", "uk",
    "uk-UA", "ur-PK", "uz-UZ", "vi-VN", "zh-CN", "zh-HK", "zh-TW", "zu-ZA",
];

/// Allowed IANA timezone names. Russia-specific zones excluded per policy.
// UI mirror: apps/ui/src/lib/locale-data.ts TIMEZONE_OPTIONS.
pub const ALLOWED_TIMEZONES: &[&str] = &[
    "UTC",
    "Africa/Abidjan",
    "Africa/Accra",
    "Africa/Addis_Ababa",
    "Africa/Algiers",
    "Africa/Cairo",
    "Africa/Casablanca",
    "Africa/Dar_es_Salaam",
    "Africa/Johannesburg",
    "Africa/Lagos",
    "Africa/Nairobi",
    "Africa/Tunis",
    "America/Anchorage",
    "America/Argentina/Buenos_Aires",
    "America/Bogota",
    "America/Cancun",
    "America/Chicago",
    "America/Denver",
    "America/Edmonton",
    "America/Halifax",
    "America/Havana",
    "America/Lima",
    "America/Los_Angeles",
    "America/Manaus",
    "America/Mexico_City",
    "America/New_York",
    "America/Panama",
    "America/Phoenix",
    "America/Santiago",
    "America/Sao_Paulo",
    "America/St_Johns",
    "America/Toronto",
    "America/Vancouver",
    "America/Winnipeg",
    "Asia/Almaty",
    "Asia/Amman",
    "Asia/Baghdad",
    "Asia/Baku",
    "Asia/Bangkok",
    "Asia/Beirut",
    "Asia/Colombo",
    "Asia/Dhaka",
    "Asia/Dubai",
    "Asia/Ho_Chi_Minh",
    "Asia/Hong_Kong",
    "Asia/Istanbul",
    "Asia/Jakarta",
    "Asia/Jerusalem",
    "Asia/Kabul",
    "Asia/Karachi",
    "Asia/Kathmandu",
    "Asia/Kolkata",
    "Asia/Kuala_Lumpur",
    "Asia/Manila",
    "Asia/Riyadh",
    "Asia/Seoul",
    "Asia/Shanghai",
    "Asia/Singapore",
    "Asia/Taipei",
    "Asia/Tashkent",
    "Asia/Tbilisi",
    "Asia/Tehran",
    "Asia/Tokyo",
    "Asia/Ulaanbaatar",
    "Asia/Yangon",
    "Asia/Yerevan",
    "Atlantic/Azores",
    "Atlantic/Reykjavik",
    "Australia/Adelaide",
    "Australia/Brisbane",
    "Australia/Darwin",
    "Australia/Hobart",
    "Australia/Melbourne",
    "Australia/Perth",
    "Australia/Sydney",
    "Europe/Amsterdam",
    "Europe/Athens",
    "Europe/Belgrade",
    "Europe/Berlin",
    "Europe/Brussels",
    "Europe/Bucharest",
    "Europe/Budapest",
    "Europe/Copenhagen",
    "Europe/Dublin",
    "Europe/Helsinki",
    "Europe/Istanbul",
    "Europe/Kyiv",
    "Europe/Lisbon",
    "Europe/London",
    "Europe/Luxembourg",
    "Europe/Madrid",
    "Europe/Oslo",
    "Europe/Paris",
    "Europe/Prague",
    "Europe/Riga",
    "Europe/Rome",
    "Europe/Sofia",
    "Europe/Stockholm",
    "Europe/Tallinn",
    "Europe/Vienna",
    "Europe/Vilnius",
    "Europe/Warsaw",
    "Europe/Zurich",
    "Indian/Maldives",
    "Indian/Mauritius",
    "Pacific/Auckland",
    "Pacific/Fiji",
    "Pacific/Guam",
    "Pacific/Honolulu",
];

/// Check whether a locale tag is in the allowed set.
pub fn is_allowed_locale(locale: &str) -> bool {
    ALLOWED_LOCALES.contains(&locale)
}

/// Check whether a timezone name is in the allowed set.
pub fn is_allowed_timezone(tz: &str) -> bool {
    ALLOWED_TIMEZONES.contains(&tz)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendLocale {
    En,
    Uk,
    Fr,
    De,
    Es,
    Zh,
}

pub fn resolve_backend_locale(locale: Option<&str>) -> BackendLocale {
    let tag = locale.unwrap_or("en").trim().as_bytes();
    if !matches!(tag.get(2), None | Some(b'-' | b'_')) {
        return BackendLocale::En;
    }
    match tag.get(..2) {
        Some(language) if language.eq_ignore_ascii_case(b"uk") => BackendLocale::Uk,
        Some(language) if language.eq_ignore_ascii_case(b"fr") => BackendLocale::Fr,
        Some(language) if language.eq_ignore_ascii_case(b"de") => BackendLocale::De,
        Some(language) if language.eq_ignore_ascii_case(b"es") => BackendLocale::Es,
        Some(language) if language.eq_ignore_ascii_case(b"zh") => {
            // A Simplified Chinese catalog must not masquerade as Traditional Chinese.
            let lower = locale
                .unwrap_or_default()
                .trim()
                .replace('_', "-")
                .to_ascii_lowercase();
            if ["zh-hant", "zh-tw", "zh-hk", "zh-mo"].iter().any(|prefix| {
                lower == *prefix
                    || lower
                        .strip_prefix(prefix)
                        .is_some_and(|rest| rest.starts_with('-'))
            }) {
                BackendLocale::En
            } else {
                BackendLocale::Zh
            }
        }
        _ => BackendLocale::En,
    }
}

/// Control-plane cancellation copy, separate from the kernel's activity catalog
/// so Engine/Wasm applications do not retain unused platform messages.
pub struct CancellationStrings {
    pub turn_cancelled: &'static str,
    pub no_turn_running: &'static str,
    pub cancellation_reason: &'static str,
    pub cancellation_message: &'static str,
}

/// Localized deterministic session-cancellation responses and notices.
pub fn cancellation_strings(locale: Option<&str>) -> &'static CancellationStrings {
    match resolve_backend_locale(locale) {
        BackendLocale::En => &CancellationStrings {
            turn_cancelled: "Turn cancelled successfully",
            no_turn_running: "No turn currently running",
            cancellation_reason: "User requested cancellation",
            cancellation_message: "User requested to cancel the work.",
        },
        BackendLocale::Uk => &CancellationStrings {
            turn_cancelled: "Виконання успішно скасовано",
            no_turn_running: "Наразі нічого не виконується",
            cancellation_reason: "Користувач попросив скасувати виконання",
            cancellation_message: "Користувач попросив скасувати роботу.",
        },
        BackendLocale::Fr => &CancellationStrings {
            turn_cancelled: "Tour annulé avec succès",
            no_turn_running: "Aucun tour en cours",
            cancellation_reason: "Annulation demandée par l’utilisateur",
            cancellation_message: "L’utilisateur a demandé l’annulation du travail.",
        },
        BackendLocale::De => &CancellationStrings {
            turn_cancelled: "Ausführung erfolgreich abgebrochen",
            no_turn_running: "Derzeit läuft keine Ausführung",
            cancellation_reason: "Abbruch vom Benutzer angefordert",
            cancellation_message: "Der Benutzer hat den Abbruch der Arbeit angefordert.",
        },
        BackendLocale::Es => &CancellationStrings {
            turn_cancelled: "Turno cancelado correctamente",
            no_turn_running: "No hay ningún turno en curso",
            cancellation_reason: "El usuario solicitó la cancelación",
            cancellation_message: "El usuario solicitó cancelar el trabajo.",
        },
        BackendLocale::Zh => &CancellationStrings {
            turn_cancelled: "已成功取消本轮执行",
            no_turn_running: "当前没有正在执行的轮次",
            cancellation_reason: "用户请求取消",
            cancellation_message: "用户请求取消工作。",
        },
    }
}

pub struct BackendStrings {
    pub working: &'static str,
    pub current_directory: &'static str,
    pub and_more_actions: &'static str,
    pub with_issues_one: &'static str,
    pub with_issues_many: &'static str,
    pub completed_tool_batch_one: &'static str,
    pub completed_tool_batch_many: &'static str,
}

const EN_STRINGS: BackendStrings = BackendStrings {
    working: "Working",
    current_directory: "current directory",
    and_more_actions: "and {count} more actions",
    with_issues_one: " with 1 issue",
    with_issues_many: " with {count} issues",
    completed_tool_batch_one: "Completed tool batch with 1 issue",
    completed_tool_batch_many: "Completed tool batch with {count} issues",
};

const UK_STRINGS: BackendStrings = BackendStrings {
    working: "Працюю",
    current_directory: "поточній директорії",
    and_more_actions: "і ще {count} дій",
    with_issues_one: " з 1 проблемою",
    with_issues_many: " з {count} проблемами",
    completed_tool_batch_one: "Пакет інструментів завершено з 1 проблемою",
    completed_tool_batch_many: "Пакет інструментів завершено з {count} проблемами",
};

const FR_STRINGS: BackendStrings = BackendStrings {
    working: "En cours",
    current_directory: "répertoire actuel",
    and_more_actions: "et {count} autres actions",
    with_issues_one: " avec 1 problème",
    with_issues_many: " avec {count} problèmes",
    completed_tool_batch_one: "Lot d’outils terminé avec 1 problème",
    completed_tool_batch_many: "Lot d’outils terminé avec {count} problèmes",
};

const DE_STRINGS: BackendStrings = BackendStrings {
    working: "In Bearbeitung",
    current_directory: "aktuelles Verzeichnis",
    and_more_actions: "und {count} weitere Aktionen",
    with_issues_one: " mit 1 Problem",
    with_issues_many: " mit {count} Problemen",
    completed_tool_batch_one: "Werkzeugstapel mit 1 Problem abgeschlossen",
    completed_tool_batch_many: "Werkzeugstapel mit {count} Problemen abgeschlossen",
};

const ES_STRINGS: BackendStrings = BackendStrings {
    working: "Trabajando",
    current_directory: "directorio actual",
    and_more_actions: "y {count} acciones más",
    with_issues_one: " con 1 problema",
    with_issues_many: " con {count} problemas",
    completed_tool_batch_one: "Lote de herramientas completado con 1 problema",
    completed_tool_batch_many: "Lote de herramientas completado con {count} problemas",
};

const ZH_STRINGS: BackendStrings = BackendStrings {
    working: "正在处理",
    current_directory: "当前目录",
    and_more_actions: "以及另外 {count} 个操作",
    with_issues_one: "，有 1 个问题",
    with_issues_many: "，有 {count} 个问题",
    completed_tool_batch_one: "工具批次已完成，有 1 个问题",
    completed_tool_batch_many: "工具批次已完成，有 {count} 个问题",
};

pub fn backend_strings(locale: Option<&str>) -> &'static BackendStrings {
    match resolve_backend_locale(locale) {
        BackendLocale::En => &EN_STRINGS,
        BackendLocale::Uk => &UK_STRINGS,
        BackendLocale::Fr => &FR_STRINGS,
        BackendLocale::De => &DE_STRINGS,
        BackendLocale::Es => &ES_STRINGS,
        BackendLocale::Zh => &ZH_STRINGS,
    }
}

/// Headline suffix for an act whose failed calls went back to the model.
/// They are issues, not errors: the turn carried on (EVE-1236).
pub fn format_issue_suffix(locale: Option<&str>, error_count: u32) -> String {
    let strings = backend_strings(locale);
    if error_count == 1 {
        strings.with_issues_one.to_string()
    } else {
        strings
            .with_issues_many
            .replace("{count}", &error_count.to_string())
    }
}

pub fn format_completed_tool_batch(locale: Option<&str>, error_count: u32) -> String {
    let strings = backend_strings(locale);
    if error_count == 1 {
        strings.completed_tool_batch_one.to_string()
    } else {
        strings
            .completed_tool_batch_many
            .replace("{count}", &error_count.to_string())
    }
}

pub fn format_more_actions(locale: Option<&str>, count: usize) -> String {
    match resolve_backend_locale(locale) {
        BackendLocale::En
        | BackendLocale::Fr
        | BackendLocale::De
        | BackendLocale::Es
        | BackendLocale::Zh => backend_strings(locale)
            .and_more_actions
            .replace("{count}", &count.to_string()),
        BackendLocale::Uk => {
            let template = match (count % 10, count % 100) {
                (1, n) if n != 11 => "і ще {count} дію",
                (2..=4, n) if !(12..=14).contains(&n) => "і ще {count} дії",
                _ => "і ще {count} дій",
            };
            template.replace("{count}", &count.to_string())
        }
    }
}

pub fn localized_tool_display_name(
    tool_name: &str,
    fallback_display_name: Option<&str>,
    locale: Option<&str>,
) -> Option<String> {
    if resolve_backend_locale(locale) != BackendLocale::Uk {
        let source = match tool_name {
            "bash" => "Shell",
            "read_file" | "session_read_file" => "Read file",
            "read_many_files" => "Read files",
            "list_directory" | "list_files" => "List files",
            "grep_files" => "Search files",
            "search" | "search_web" => "Search web",
            name if name.ends_with("__search") => "Search",
            "write_file" => "Write file",
            "edit_file" | "replace_in_file" => "Edit file",
            "append_file" => "Append to file",
            "move_file" => "Move file",
            "delete_file" => "Delete file",
            "mkdir" => "Create directory",
            "stat_file" => "File information",
            "secret_store" => "Secret Store",
            "kv_store" => "Key Value Store",
            "spawn_agent" => "Start agent",
            "write_todos" => "Task list",
            "setup_connection" => "Set up connection",
            _ => return fallback_display_name.map(ToOwned::to_owned),
        };
        let translated = runtime_text(locale, source);
        return if translated != source {
            Some(translated.to_string())
        } else {
            fallback_display_name.map(ToOwned::to_owned)
        };
    }

    let localized = match tool_name {
        "bash" => Some("Командний рядок"),
        "read_file" | "session_read_file" => Some("Читання файла"),
        "read_many_files" => Some("Читання файлів"),
        "list_directory" | "list_files" => Some("Список файлів"),
        "grep_files" => Some("Пошук у файлах"),
        "search" | "search_web" => Some("Пошук у вебі"),
        name if name.ends_with("__search") => Some("Пошук"),
        "write_file" => Some("Запис файла"),
        "edit_file" | "replace_in_file" => Some("Редагування файла"),
        "append_file" => Some("Дописування у файл"),
        "move_file" => Some("Переміщення файла"),
        "delete_file" => Some("Видалення файла"),
        "mkdir" => Some("Створення директорії"),
        "stat_file" => Some("Інформація про файл"),
        "secret_store" => Some("Сховище секретів"),
        "kv_store" => Some("Сховище значень"),
        "spawn_agent" => Some("Запуск агента"),
        "write_todos" => Some("Список задач"),
        "setup_connection" => Some("Налаштування підключення"),
        _ => None,
    };

    localized
        .map(str::to_string)
        .or_else(|| fallback_display_name.map(ToOwned::to_owned))
}

#[cfg(test)]
mod display_localization_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn fields_resolve_independently_and_other_languages_are_supported() {
        let entries: EntityLocalizations = serde_json::from_value(json!({
            "UK_ua": {"display_name": "Підтримка"},
            "uk": {"description": "Допомога"},
            "en": {"display_name": "Support", "description": "Help"},
            "de": {"display_name": "Hilfe"}
        }))
        .unwrap();
        assert_eq!(
            entries.resolve("uk-UA", |e| e.display_name.as_deref()),
            Some("Підтримка")
        );
        assert_eq!(
            entries.resolve("uk-UA", |e| e.description.as_deref()),
            Some("Допомога")
        );
        assert_eq!(
            entries.resolve("de-AT", |e| e.display_name.as_deref()),
            Some("Hilfe")
        );
        assert_eq!(
            entries.resolve("fr-FR", |e| e.description.as_deref()),
            Some("Help")
        );
    }

    #[test]
    fn invalid_and_ambiguous_overlays_fail_at_the_boundary() {
        for value in [
            json!({"": {"display_name":"Name"}}),
            json!({"uk--UA": {"display_name":"Name"}}),
            json!({"uk": {"display_name":" "}}),
            json!({"uk": {"instructions":"Injected instructions"}}),
            json!({"uk-UA": {}, "UK_ua": {}}),
            json!({"uk": {"display_name":"a".repeat(256)}}),
        ] {
            assert!(
                serde_json::from_value::<EntityLocalizations>(value.clone()).is_err(),
                "{value}"
            );
        }
        let oversized = (0..65)
            .map(|i| (format!("en-{i:03}"), json!({})))
            .collect::<serde_json::Map<_, _>>();
        assert!(serde_json::from_value::<EntityLocalizations>(json!(oversized)).is_err());
    }

    #[test]
    fn backend_locale_requires_a_whole_language_prefix() {
        for locale in ["uk", "uk-UA", "UK_ua", " uk-UA "] {
            assert_eq!(resolve_backend_locale(Some(locale)), BackendLocale::Uk);
        }
        for locale in ["", "u", "ukulele", "ukrainian", "uké", "éuk-UA", "ja"] {
            assert_eq!(resolve_backend_locale(Some(locale)), BackendLocale::En);
        }
        assert_eq!(resolve_backend_locale(None), BackendLocale::En);
    }

    #[test]
    fn ukrainian_action_counts_use_the_final_digits() {
        for (count, ending) in [
            (1, "дію"),
            (2, "дії"),
            (5, "дій"),
            (11, "дій"),
            (14, "дій"),
            (21, "дію"),
            (22, "дії"),
            (111, "дій"),
        ] {
            assert_eq!(
                format_more_actions(Some("uk-UA"), count),
                format!("і ще {count} {ending}")
            );
        }
        assert_eq!(resolve_backend_locale(Some("UK_ua")), BackendLocale::Uk);
        assert_eq!(
            cancellation_strings(Some("UK_ua")).turn_cancelled,
            "Виконання успішно скасовано"
        );
        assert_eq!(
            cancellation_strings(Some("ja")).turn_cancelled,
            "Turn cancelled successfully"
        );
    }
}

#[cfg(test)]
mod additional_locale_tests {
    use super::*;

    #[test]
    fn activity_preserves_instruction_filenames() {
        for locale in ["fr", "de", "es", "zh-CN", "en-GB"] {
            assert!(runtime_text(Some(locale), "Read AGENTS.md").contains("AGENTS.md"));
        }
    }

    #[test]
    fn regional_language_tags_resolve_without_prefix_false_positives() {
        for (tag, expected) in [
            ("fr-CA", BackendLocale::Fr),
            ("DE_at", BackendLocale::De),
            ("es-MX", BackendLocale::Es),
            ("zh-Hans-CN", BackendLocale::Zh),
            ("en-GB", BackendLocale::En),
            ("french", BackendLocale::En),
            ("zh-Hant-TW", BackendLocale::En),
            ("constructor", BackendLocale::En),
        ] {
            assert_eq!(resolve_backend_locale(Some(tag)), expected, "{tag}");
        }
    }

    #[test]
    fn cancellation_and_activity_are_translated_before_arguments_are_added() {
        for (locale, notice, reading) in [
            ("fr-CA", "Tour annulé avec succès", "Lecture de"),
            ("de-DE", "Ausführung erfolgreich abgebrochen", "Lese"),
            ("es-MX", "Turno cancelado correctamente", "Leyendo"),
            ("zh-CN", "已成功取消本轮执行", "正在读取"),
        ] {
            assert_eq!(cancellation_strings(Some(locale)).turn_cancelled, notice);
            assert_eq!(runtime_text(Some(locale), "Reading"), reading);
            assert_eq!(
                runtime_text(Some(locale), "Customer document {name}"),
                "Customer document {name}"
            );
        }
    }
}
