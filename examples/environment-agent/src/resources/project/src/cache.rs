//! Store responses so repeat requests stay cheap.

use std::collections::HashMap;

#[derive(Default)]
pub struct Cache {
    entries: HashMap<String, String>,
}

impl Cache {
    pub fn get(&self, key: &str) -> Option<&String> {
        self.entries.get(key)
    }

    pub fn put(&mut self, key: String, value: String) {
        self.entries.insert(key, value);
    }
}
