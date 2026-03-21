//! Backend localization helpers for deterministic platform-authored strings.
//!
//! Decisions:
//! - Keep locale fallback simple: Ukrainian (`uk*`) or default English.
//! - Centralize string catalogs here so backend-authored copy stays externalized
//!   as we add more locales.

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
