//! Backend localization helpers for deterministic platform-authored strings.
//!
//! Decisions:
//! - Keep locale fallback simple: Ukrainian (`uk*`) or default English.
//! - Centralize string catalogs here so backend-authored copy stays externalized
//!   as we add more locales.
//! - Allowed locale and timezone lists are the single source of truth for the
//!   backend. The UI mirrors these in `apps/ui/src/lib/locale-data.ts`.

/// Allowed BCP 47 locale tags. Russia excluded per policy.
/// UI mirror: `apps/ui/src/lib/locale-data.ts` LOCALE_OPTIONS.
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
/// UI mirror: `apps/ui/src/lib/locale-data.ts` TIMEZONE_OPTIONS.
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
}

pub fn resolve_backend_locale(locale: Option<&str>) -> BackendLocale {
    match locale
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_ascii_lowercase())
    {
        Some(value) if value == "uk" || value.starts_with("uk-") => BackendLocale::Uk,
        _ => BackendLocale::En,
    }
}

pub struct BackendStrings {
    pub working: &'static str,
    pub current_directory: &'static str,
    pub and_more_actions: &'static str,
    pub with_errors_one: &'static str,
    pub with_errors_many: &'static str,
    pub completed_tool_batch_one: &'static str,
    pub completed_tool_batch_many: &'static str,
}

const EN_STRINGS: BackendStrings = BackendStrings {
    working: "Working",
    current_directory: "current directory",
    and_more_actions: "and {count} more actions",
    with_errors_one: " with 1 error",
    with_errors_many: " with {count} errors",
    completed_tool_batch_one: "Completed tool batch with 1 error",
    completed_tool_batch_many: "Completed tool batch with {count} errors",
};

const UK_STRINGS: BackendStrings = BackendStrings {
    working: "Працюю",
    current_directory: "поточній директорії",
    and_more_actions: "і ще {count} дій",
    with_errors_one: " з 1 помилкою",
    with_errors_many: " з {count} помилками",
    completed_tool_batch_one: "Пакет інструментів завершено з 1 помилкою",
    completed_tool_batch_many: "Пакет інструментів завершено з {count} помилками",
};

pub fn backend_strings(locale: Option<&str>) -> &'static BackendStrings {
    match resolve_backend_locale(locale) {
        BackendLocale::En => &EN_STRINGS,
        BackendLocale::Uk => &UK_STRINGS,
    }
}

pub fn format_error_suffix(locale: Option<&str>, error_count: u32) -> String {
    let strings = backend_strings(locale);
    if error_count == 1 {
        strings.with_errors_one.to_string()
    } else {
        strings
            .with_errors_many
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
        BackendLocale::En => backend_strings(locale)
            .and_more_actions
            .replace("{count}", &count.to_string()),
        BackendLocale::Uk => {
            let template = match count {
                1 => "і ще 1 дію",
                2..=4 => "і ще {count} дії",
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
        return fallback_display_name.map(ToOwned::to_owned);
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
        "spawn_subagent" => Some("Запуск субагента"),
        "get_subagents" => Some("Субагенти"),
        "message_subagent" => Some("Повідомлення субагенту"),
        "write_todos" => Some("Список задач"),
        "setup_connection" => Some("Налаштування підключення"),
        _ => None,
    };

    localized
        .map(str::to_string)
        .or_else(|| fallback_display_name.map(ToOwned::to_owned))
}
