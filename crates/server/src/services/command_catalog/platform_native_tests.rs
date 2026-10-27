use crate::domains::common::CommandDescriptor;

#[test]
fn native_platform_catalog_matches_authoritative_inventory() {
    let mut entries: Vec<_> = inventory::iter::<CommandDescriptor>
        .into_iter()
        .filter(|desc| {
            let meta = (desc.meta)();
            !meta.is_internal() && !meta.is_retired() && !meta.path.starts_with("/v1/durable/")
        })
        .map(|desc| {
            let meta = (desc.meta)();
            serde_json::json!({
                "name": meta.name, "category": meta.category,
                "description": meta.description, "read_only": (desc.read_only)(),
                "input_schema": (desc.param_schema)(),
            })
        })
        .collect();
    entries.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
    let generated = format!(
        "[\n{}\n]\n",
        entries
            .iter()
            .map(|entry| serde_json::to_string(entry).unwrap())
            .collect::<Vec<_>>()
            .join(",\n")
    );
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../capabilities/src/capabilities/platform_native/operations.json"
    );
    if std::env::var_os("UPDATE_PLATFORM_NATIVE_CATALOG").is_some() {
        std::fs::write(path, generated).unwrap();
        return;
    }
    let checked_in: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    assert_eq!(
        checked_in,
        serde_json::json!(entries),
        "Run UPDATE_PLATFORM_NATIVE_CATALOG=1 cargo test -p everruns-server --lib native_platform_catalog_matches_authoritative_inventory"
    );
}
