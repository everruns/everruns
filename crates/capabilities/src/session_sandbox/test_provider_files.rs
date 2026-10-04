use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

type FileKey = (String, String);

static FILES: LazyLock<Mutex<HashMap<FileKey, Vec<u8>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

pub(super) fn clear(external_id: &str) {
    FILES
        .lock()
        .unwrap()
        .retain(|(instance, _), _| instance != external_id);
}

pub(super) fn write(external_id: &str, path: &str, content: &[u8]) {
    FILES.lock().unwrap().insert(
        (external_id.to_string(), path.to_string()),
        content.to_vec(),
    );
}

pub(super) fn read(external_id: &str, path: &str) -> String {
    FILES
        .lock()
        .unwrap()
        .get(&(external_id.to_string(), path.to_string()))
        .map(|content| String::from_utf8_lossy(content).into_owned())
        .unwrap_or_else(|| "data".to_string())
}

pub(super) fn command_output(external_id: &str, command: &str) -> String {
    command
        .strip_prefix("cat ")
        .map(|path| read(external_id, path.trim()))
        .unwrap_or_else(|| "ok".to_string())
}
