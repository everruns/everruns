//! Issue HTTP requests.

pub struct Client {
    pub timeout_secs: u64,
}

impl Client {
    pub fn new() -> Self {
        // TODO: read the timeout from configuration instead of hardcoding it.
        Self { timeout_secs: 30 }
    }

    pub fn get(&self, url: &str) -> Result<String, String> {
        if url.is_empty() {
            return Err("empty url".into());
        }
        // TODO: follow redirects up to a bounded depth.
        // TODO: surface the status code rather than collapsing it into a string.
        Ok(format!("GET {url}"))
    }
}
