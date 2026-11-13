use super::TestServer;
use serde_json::Value;

impl TestServer {
    pub async fn all_capabilities(&self) -> Vec<Value> {
        let mut capabilities = Vec::new();
        loop {
            // Exercise pagination even when the built-in catalog changes size.
            let page = self
                .get(&format!(
                    "/v1/capabilities?offset={}&limit=25",
                    capabilities.len()
                ))
                .await
                .assert_success()
                .json_value();
            let data = page["data"].as_array().unwrap();
            let total = page["total"].as_u64().unwrap() as usize;
            assert!(!data.is_empty() || capabilities.len() >= total);
            capabilities.extend(data.iter().cloned());
            if capabilities.len() >= total {
                return capabilities;
            }
        }
    }
}
