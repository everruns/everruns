use crate::runtime::tool_narration::{ToolNarrationPhase, narrate_labeled_action};
use serde_json::Value;

pub(super) fn render(arguments: &Value, phase: ToolNarrationPhase, locale: Option<&str>) -> String {
    let (english, ukrainian) = match arguments.get("action").and_then(Value::as_str) {
        Some("screenshot") => (
            (
                "Taking screenshot",
                "Took screenshot",
                "Could not take screenshot",
            ),
            (
                "Роблю знімок екрана",
                "Зробив знімок екрана",
                "Не вдалося зробити знімок екрана",
            ),
        ),
        Some("left_click" | "right_click" | "middle_click" | "double_click" | "triple_click") => (
            (
                "Clicking on screen",
                "Clicked on screen",
                "Could not click on screen",
            ),
            (
                "Натискаю на екрані",
                "Натиснув на екрані",
                "Не вдалося натиснути на екрані",
            ),
        ),
        Some("left_click_drag") => (
            (
                "Dragging on screen",
                "Dragged on screen",
                "Could not drag on screen",
            ),
            (
                "Перетягую на екрані",
                "Перетягнув на екрані",
                "Не вдалося перетягнути на екрані",
            ),
        ),
        Some("left_mouse_down" | "left_mouse_up") => (
            (
                "Pressing mouse button",
                "Pressed mouse button",
                "Could not press mouse button",
            ),
            (
                "Натискаю кнопку миші",
                "Натиснув кнопку миші",
                "Не вдалося натиснути кнопку миші",
            ),
        ),
        Some("cursor_position") => (
            (
                "Reading pointer position",
                "Read pointer position",
                "Could not read pointer position",
            ),
            (
                "Визначаю положення вказівника",
                "Визначив положення вказівника",
                "Не вдалося визначити положення вказівника",
            ),
        ),
        Some("zoom") => (
            (
                "Zooming into screen",
                "Zoomed into screen",
                "Could not zoom into screen",
            ),
            (
                "Збільшую частину екрана",
                "Збільшив частину екрана",
                "Не вдалося збільшити частину екрана",
            ),
        ),
        Some("hold_key") => (
            ("Holding key", "Held key", "Could not hold key"),
            (
                "Утримую клавішу",
                "Утримав клавішу",
                "Не вдалося утримати клавішу",
            ),
        ),
        Some("mouse_move") => (
            ("Moving pointer", "Moved pointer", "Could not move pointer"),
            (
                "Переміщую вказівник",
                "Перемістив вказівник",
                "Не вдалося перемістити вказівник",
            ),
        ),
        Some("scroll") => (
            ("Scrolling page", "Scrolled page", "Could not scroll page"),
            (
                "Прокручую сторінку",
                "Прокрутив сторінку",
                "Не вдалося прокрутити сторінку",
            ),
        ),
        Some("type") => (
            ("Typing text", "Typed text", "Could not type text"),
            ("Вводжу текст", "Ввів текст", "Не вдалося ввести текст"),
        ),
        Some("key") => (
            ("Pressing keys", "Pressed keys", "Could not press keys"),
            (
                "Натискаю клавіші",
                "Натиснув клавіші",
                "Не вдалося натиснути клавіші",
            ),
        ),
        Some("wait") => (
            (
                "Waiting for screen",
                "Waited for screen",
                "Could not wait for screen",
            ),
            (
                "Очікую на екран",
                "Зачекав на екран",
                "Не вдалося зачекати на екран",
            ),
        ),
        Some("navigate") => (
            (
                "Navigating to page",
                "Navigated to page",
                "Could not navigate to page",
            ),
            (
                "Переходжу на сторінку",
                "Перейшов на сторінку",
                "Не вдалося перейти на сторінку",
            ),
        ),
        _ => (
            (
                "Operating computer",
                "Operated computer",
                "Could not operate computer",
            ),
            (
                "Керую комп'ютером",
                "Керував комп'ютером",
                "Не вдалося керувати комп'ютером",
            ),
        ),
    };
    // Typed text can be a password. Navigation is the only action with a display detail.
    let keys: &[&str] = if arguments.get("action").and_then(Value::as_str) == Some("navigate") {
        &["url"]
    } else {
        &[]
    };
    narrate_labeled_action(arguments, phase, locale, english, ukrainian, keys)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn computer_actions_never_echo_typed_credentials() {
        for action in [
            "screenshot",
            "left_click",
            "right_click",
            "middle_click",
            "double_click",
            "triple_click",
            "left_click_drag",
            "mouse_move",
            "scroll",
            "type",
            "key",
            "wait",
            "navigate",
            "unknown",
        ] {
            for phase in [
                ToolNarrationPhase::Started,
                ToolNarrationPhase::Waiting,
                ToolNarrationPhase::Completed,
                ToolNarrationPhase::Failed,
            ] {
                for locale in [None, Some("uk-UA")] {
                    let line = render(
                        &json!({"action":action,"text":"PRIVATE","url":"https://user:PRIVATE@example.com?token=PRIVATE"}),
                        phase,
                        locale,
                    );
                    assert!(!line.contains("PRIVATE"), "{line}");
                }
            }
        }
        assert_eq!(
            render(
                &json!({"action":"type","text":"PRIVATE"}),
                ToolNarrationPhase::Completed,
                None
            ),
            "Typed text"
        );
        assert_eq!(
            render(
                &json!({"action":"navigate","url":"https://user:PRIVATE@example.com?token=PRIVATE"}),
                ToolNarrationPhase::Completed,
                None
            ),
            "Navigated to page: example.com"
        );
    }
}
