//! Explicit native async opt-in. The host owns persistence and execution policy.
use async_trait::async_trait;
use everruns_core::Capability;
use serde_json::{Value, json};
use std::collections::BTreeMap;

pub struct NativeAsyncToolsCapability;
fn selected(config: &Value) -> Result<BTreeMap<String, Option<Value>>, String> {
    if config.is_null() || config == &json!({}) {
        return Ok(BTreeMap::from([("web_fetch".into(), None)]));
    }
    let tools = config
        .get("tools")
        .ok_or("native async requires a tools map")?;
    let tools: BTreeMap<String, Option<Value>> = serde_json::from_value(tools.clone())
        .map_err(|_| "native async tools must map names to null or a custom format")?;
    if tools.is_empty() || tools.keys().any(|name| name.is_empty()) {
        return Err("native async tools cannot be empty".into());
    }
    if tools.values().flatten().any(|format| !format.is_object()) {
        return Err("custom tool formats must be objects".into());
    }
    Ok(tools)
}
#[async_trait]
impl Capability for NativeAsyncToolsCapability {
    fn id(&self) -> &str {
        "native_async_tools"
    }
    fn name(&self) -> &str {
        "Native Async Tools"
    }
    fn description(&self) -> &str {
        "Run selected read-only lookups while a supported model continues reasoning. Requires durable worker storage."
    }
    fn category(&self) -> Option<&str> {
        Some("Optimization")
    }
    fn native_async_tools(&self, config: &Value) -> Option<BTreeMap<String, Option<Value>>> {
        selected(config).ok()
    }
    fn validate_config(&self, config: &Value) -> Result<(), String> {
        selected(config).map(|_| ())
    }
    fn config_schema(&self) -> Option<Value> {
        Some(
            json!({"type":"object","properties":{"tools":{"type":"object","description":"Tool names mapped to null for functions, or a custom tool format object.","default":{"web_fetch":null},"additionalProperties":{"type":["object","null"]}}}}),
        )
    }
}
