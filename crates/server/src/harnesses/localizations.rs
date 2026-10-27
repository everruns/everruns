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
    EntityLocalizations(std::collections::BTreeMap::from([(
        "uk".into(),
        EntityLocalization {
            display_name: Some(display_name.into()),
            description: Some(description.into()),
        },
    )]))
}
