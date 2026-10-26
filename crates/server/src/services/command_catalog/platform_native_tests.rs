//! Production workers consume this data without linking server domain records.
//! Keep the captured catalog complete and exclude infrastructure-only commands.

use crate::domains::common::CommandDescriptor;
use serde_json::{Value, json};

#[test]
fn platform_capability_search_context_is_documented_in_tool_hints() {
    let schema = serde_json::to_value(
        <everruns_contracts::tool_types::ToolHints as utoipa::PartialSchema>::schema(),
    )
    .unwrap();
    let published: Value =
        serde_json::from_str(include_str!("../../../../../docs/api/openapi.json")).unwrap();
    assert_eq!(
        schema["properties"]["capability_description"],
        published["components"]["schemas"]["ToolHints"]["properties"]["capability_description"]
    );
    assert!(schema["properties"]["capability_description"].is_object());
}

#[test]
fn the_native_platform_catalog_matches_inventory() {
    let mut operations: Vec<Value> = inventory::iter::<CommandDescriptor>
        .into_iter()
        .filter(|desc| super::catalog::is_exposed(&(desc.meta)()))
        .map(|desc| {
            let meta = (desc.meta)();
            json!({
                "name": meta.name,
                "description": meta.description,
                "read_only": (desc.read_only)(),
                "input_schema": (desc.param_schema)(),
            })
        })
        .collect();
    operations.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../capabilities/src/capabilities/platform_native/operations.json"
    );
    if std::env::var_os("UPDATE_PLATFORM_NATIVE_CATALOG").is_some() {
        std::fs::write(
            path,
            serde_json::to_string_pretty(&operations).unwrap() + "\n",
        )
        .unwrap();
        return;
    }
    let actual: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    assert_eq!(
        actual,
        json!(operations),
        "Native platform catalog is stale. Run UPDATE_PLATFORM_NATIVE_CATALOG=1 cargo test -p everruns-server --lib the_native_platform_catalog_matches_inventory"
    );
    let portable: Value = serde_json::to_value(
        everruns_capabilities::capabilities::platform_native::operations()
            .iter()
            .map(|operation| operation.name.as_str())
            .collect::<Vec<_>>(),
    )
    .unwrap();
    assert_eq!(
        portable,
        json!(operations.iter().map(|op| &op["name"]).collect::<Vec<_>>())
    );
}
