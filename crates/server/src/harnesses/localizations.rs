//! Maintained display overlays for platform-supplied resources.
use everruns_contracts::runtime::localization::{EntityLocalization, EntityLocalizations};

pub(crate) fn for_builtin(name: &str) -> EntityLocalizations {
    let (display_name, description) = match name {
        "base" => ("Основа", "Порожня основа без додаткових можливостей."),
        "conversation" => (
            "Розмова",
            "Спілкування з агентом і базові можливості розмови.",
        ),
        "worker" => (
            "Виконавець",
            "Основа для агентів, які виконують завдання та працюють з інструментами.",
        ),
        "bashkit-worker" => (
            "Виконавець Bashkit",
            "Виконання завдань у віртуальному робочому просторі Bashkit.",
        ),
        "sandbox-worker" => (
            "Виконавець у пісочниці",
            "Виконання завдань у повноцінному ізольованому середовищі.",
        ),
        "worker-base" => ("Базовий виконавець", "Застаріла базова обв’язка виконавця."),
        "generic" => (
            "Універсальна",
            "Універсальна базова обв’язка з файловою системою та сховищем секретів.",
        ),
        "coding" => (
            "Програмування",
            "Обв’язка для роботи з кодом в ізольованому середовищі.",
        ),
        "coding-session-sandbox" => (
            "Програмування в пісочниці",
            "Робота з кодом в ізольованому середовищі сесії.",
        ),
        "data-analyst" => (
            "Аналітик даних",
            "Дослідження даних, обчислення та підготовка звітів.",
        ),
        "platform-chat" => ("Чат платформи", "Керування вашою організацією Everruns."),
        "dad-jokes-agent" => ("Агент жартів", "Розповідає короткі доброзичливі жарти."),
        "research-agent" => ("Дослідник", "Досліджує теми та збирає відомості з джерел."),
        "microsoft-learn-assistant" => (
            "Помічник Microsoft Learn",
            "Допомагає знаходити та використовувати документацію Microsoft Learn.",
        ),
        "python-coder" => (
            "Програміст Python",
            "Допомагає писати та виконувати код Python.",
        ),
        "shell-assistant" => (
            "Помічник командного рядка",
            "Допомагає виконувати завдання в командному рядку.",
        ),
        "image-studio-agent" => ("Студія зображень", "Створює та редагує зображення."),
        "cursor-agent-manager" => (
            "Керівник агентів Cursor",
            "Керує агентами програмування Cursor.",
        ),
        "e2b-coder" => ("Програміст E2B", "Працює з кодом у пісочниці E2B."),
        "sprites-coder" => ("Програміст Sprites", "Працює з кодом у середовищі Sprites."),
        _ => return EntityLocalizations::default(),
    };
    let mut entries = EntityLocalizations(std::collections::BTreeMap::from([(
        "uk".into(),
        EntityLocalization {
            display_name: Some(display_name.into()),
            description: Some(description.into()),
        },
    )]));
    let (name, description) = match name {
        "base" => (
            "Base",
            "An empty foundation with no additional capabilities.",
        ),
        "conversation" => (
            "Conversation",
            "Agent conversations and essential chat capabilities.",
        ),
        "worker" => (
            "Worker",
            "A foundation for agents that perform tasks and use tools.",
        ),
        "bashkit-worker" => (
            "Bashkit Worker",
            "Run tasks in a virtual Bashkit workspace.",
        ),
        "sandbox-worker" => (
            "Sandbox Worker",
            "Run tasks in a fully isolated environment.",
        ),
        "worker-base" => ("Base Worker", "Legacy base worker harness."),
        "generic" => (
            "Generic",
            "A general-purpose base harness with a file system and secret store.",
        ),
        "coding" => (
            "Coding",
            "A harness for working with code in an isolated environment.",
        ),
        "coding-session-sandbox" => (
            "Sandbox Coding",
            "Work with code in an isolated session environment.",
        ),
        "data-analyst" => (
            "Data Analyst",
            "Explore data, perform calculations and prepare reports.",
        ),
        "platform-chat" => ("Platform Chat", "Manage your Everruns organization."),
        "dad-jokes-agent" => ("Dad Jokes Agent", "Tells short, friendly jokes."),
        "research-agent" => (
            "Researcher",
            "Research topics and gather information from sources.",
        ),
        "microsoft-learn-assistant" => (
            "Microsoft Learn Assistant",
            "Find and use Microsoft Learn documentation.",
        ),
        "python-coder" => ("Python Coder", "Write and run Python code."),
        "shell-assistant" => ("Shell Assistant", "Perform tasks in the command line."),
        "image-studio-agent" => ("Image Studio", "Create and edit images."),
        "cursor-agent-manager" => ("Cursor Agent Manager", "Manage Cursor coding agents."),
        "e2b-coder" => ("E2B Coder", "Work with code in an E2B sandbox."),
        "sprites-coder" => (
            "Sprites Coder",
            "Work with code in the Sprites environment.",
        ),
        _ => return entries,
    };
    for locale in crate::services::capability_localizations::ADDITIONAL_LOCALES {
        entries.0.insert(
            locale.into(),
            EntityLocalization {
                display_name: Some(
                    crate::services::capability_localizations::translated(locale, name).into(),
                ),
                description: Some(
                    crate::services::capability_localizations::translated(locale, description)
                        .into(),
                ),
            },
        );
    }
    entries
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn builtins_carry_languages_without_rewriting_unknown_resources() {
        let names = for_builtin("conversation");
        for locale in ["uk", "fr", "de", "es", "zh"] {
            assert!(
                names
                    .0
                    .get(locale)
                    .and_then(|entry| entry.display_name.as_deref())
                    .is_some()
            );
        }
        assert!(for_builtin("customer-harness").is_empty());
    }
}
