use super::*;
use crate::session_file::SessionFile;

impl AgentPackage {
    /// Apply host edits to a previously loaded repository package. Unchanged authored
    /// fields and external instruction/file paths survive host normalization. Only
    /// selected package assets are returned; the host must protect unrelated files.
    pub fn edit_entries(
        entries: BTreeMap<String, Vec<u8>>,
        manifest: &str,
        previous: &Self,
        desired: &Self,
    ) -> Result<BTreeMap<String, Vec<u8>>> {
        Self::from_manifest_entries(entries.clone(), manifest)?;
        let format = Format::from_extension(Path::new(manifest));
        let text = std::str::from_utf8(&entries[manifest]).map_err(|e| error(manifest, e))?;
        let original = Self::parse(text, format)?;
        let mut materialized = original.clone();
        let mut sources = BTreeMap::new();
        materialized.resolve_assets_tracked(entries.clone(), true, &mut sources)?;
        let before = previous.canonical()?;
        let after = desired.canonical()?;
        let mut authored =
            serde_json::to_value(&original.manifest).map_err(|e| error(manifest, e))?;
        let next = serde_json::to_value(&desired.manifest).map_err(|e| error(manifest, e))?;
        let keys: BTreeSet<_> = before
            .as_object()
            .unwrap()
            .keys()
            .chain(after.as_object().unwrap().keys())
            .cloned()
            .collect();
        let mut output = entries.clone();
        let mut changed_manifest = false;
        for key in keys {
            if key == "files" || key == "instructions" || before.get(&key) == after.get(&key) {
                continue;
            }
            changed_manifest = true;
            let original = authored.get(&key).cloned().unwrap_or(Value::Null);
            authored.as_object_mut().unwrap().remove(&key);
            if let Some(value) = next.get(&key) {
                authored[&key] = merge_authored(
                    &original,
                    before.get(&key).unwrap_or(&Value::Null),
                    after.get(&key).unwrap_or(&Value::Null),
                    value,
                );
            }
        }
        if before["instructions"] != after["instructions"] {
            if original.manifest.instructions.is_empty() {
                let path = original
                    .manifest
                    .instructions_file
                    .as_deref()
                    .unwrap_or("instructions.md");
                output.insert(
                    path.into(),
                    desired.manifest.instructions.as_bytes().to_vec(),
                );
            } else {
                authored["instructions"] = json_string(&desired.manifest.instructions);
                changed_manifest = true;
            }
        }
        if before["files"] != after["files"] {
            // Keep source paths (including remapped globs and legacy skill roots).
            // A changed file set becomes explicit, so deleted assets cannot be
            // rediscovered by a glob or implicit companion-file discovery.
            let mut declarations = Vec::new();
            let mut writes = BTreeMap::new();
            let instruction_path = if original.manifest.instructions.is_empty() {
                Some(
                    original
                        .manifest
                        .instructions_file
                        .as_deref()
                        .unwrap_or("instructions.md"),
                )
            } else {
                None
            };
            for file in desired.files()? {
                if let Some(source) = sources.get(&file.path) {
                    if source == manifest || Some(source.as_str()) == instruction_path {
                        return Err(error(
                            source,
                            "runtime asset overlaps the manifest or instructions",
                        ));
                    }
                    let bytes = SessionFile::decode_content(&file.content, &file.encoding)
                        .map_err(|e| error(source, e))?;
                    if writes
                        .insert(source.clone(), bytes.clone())
                        .is_some_and(|old| old != bytes)
                    {
                        return Err(error(
                            source,
                            "multiple workspace paths disagree about the same asset",
                        ));
                    }
                    declarations.push(File::Source(FileSource {
                        source: source.clone(),
                        path: Some(file.path),
                        is_readonly: file.is_readonly,
                    }));
                } else {
                    // New and previously inline assets remain inline: never claim
                    // or overwrite an unrelated repository file at a matching path.
                    declarations.push(File::Inline(file));
                }
            }
            let topology_unchanged = before["files"].as_object().unwrap().len()
                == after["files"].as_object().unwrap().len()
                && before["files"]
                    .as_object()
                    .unwrap()
                    .iter()
                    .all(|(path, value)| {
                        after["files"]
                            .get(path)
                            .is_some_and(|next| next["is_readonly"] == value["is_readonly"])
                    });
            let all_external = desired
                .files()?
                .iter()
                .all(|file| sources.contains_key(&file.path));
            if topology_unchanged && all_external {
                output.extend(writes);
            } else {
                for source in sources.values() {
                    if source != manifest && Some(source.as_str()) != instruction_path {
                        output.remove(source);
                    }
                }
                output.extend(writes);
                authored["files"] =
                    serde_json::to_value(declarations).map_err(|e| error(manifest, e))?;
                authored["skills"] = serde_json::json!([]);
                if before["capabilities"] == after["capabilities"] {
                    authored["capabilities"] =
                        serde_json::to_value(&materialized.manifest.capabilities)
                            .map_err(|e| error(manifest, e))?;
                }
                changed_manifest = true;
            }
        }
        if changed_manifest {
            let manifest_value: Manifest =
                serde_json::from_value(authored).map_err(|e| error(manifest, e))?;
            output.insert(
                manifest.into(),
                render_authored(
                    &manifest_value,
                    format,
                    original.files_declared || before["files"] != after["files"],
                )?
                .into_bytes(),
            );
        }
        // Reparse the complete result before the host writes any Git objects.
        let reparsed = Self::from_manifest_entries(output.clone(), manifest)?;
        if before["files"] == after["files"]
            && reparsed.canonical()?["files"] != materialized.canonical()?["files"]
        {
            return Err(error(
                "files",
                "instruction edit would also change a runtime asset; edit the shared source in GitHub",
            ));
        }
        Ok(output)
    }
}

// Host exports normalize defaults and publication. Preserve authored values at
// leaves the host did not change, rather than replacing an entire config map.
fn merge_authored(original: &Value, before: &Value, after: &Value, next: &Value) -> Value {
    if before == after {
        return original.clone();
    }
    if let (Some(before), Some(after), Some(next), Some(original)) = (
        before.as_object(),
        after.as_object(),
        next.as_object(),
        original.as_object(),
    ) {
        let mut result = original.clone();
        let keys: BTreeSet<_> = before.keys().chain(after.keys()).collect();
        for key in keys {
            if before.get(key) == after.get(key) {
                continue;
            }
            let old = result.remove(key).unwrap_or(Value::Null);
            if let Some(value) = next.get(key) {
                result.insert(
                    key.clone(),
                    merge_authored(
                        &old,
                        before.get(key).unwrap_or(&Value::Null),
                        after.get(key).unwrap_or(&Value::Null),
                        value,
                    ),
                );
            }
        }
        Value::Object(result)
    } else {
        next.clone()
    }
}

fn render_authored(manifest: &Manifest, format: Format, explicit_files: bool) -> Result<String> {
    let rendered = AgentPackage::render_manifest(manifest, format)?;
    if !explicit_files || !manifest.initial_files.is_empty() {
        return Ok(rendered);
    }
    // Omitting an explicit empty selection would opt into legacy companion-file
    // discovery and accidentally claim unrelated repository files.
    Ok(match format {
        Format::Toml => format!("files = []\n{rendered}"),
        Format::Yaml => format!("files: []\n{rendered}"),
        Format::Markdown | Format::Auto => rendered.replacen("---\n", "---\nfiles: []\n", 1),
        Format::Json => {
            let mut value: Value =
                serde_json::from_str(&rendered).map_err(|e| error("manifest", e))?;
            value["files"] = serde_json::json!([]);
            serde_json::to_string_pretty(&value).map_err(|e| error("manifest", e))?
        }
    })
}

fn json_string(value: &str) -> Value {
    Value::String(value.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> BTreeMap<String, Vec<u8>> {
        [
            ("custom.toml", b"# retain layout\nschema_version=1\nname='helper'\ninstructions_file='prompt.md'\nfiles=[{source='assets',path='/work'}]\n".to_vec()),
            ("prompt.md", b"Original instructions".to_vec()),
            ("assets/data.bin", vec![0, 255, 7]),
            ("README.md", b"Unrelated repository file".to_vec()),
        ].into_iter().map(|(p,b)|(p.into(),b)).collect()
    }

    #[test]
    fn external_instruction_edit_preserves_manifest_bytes_and_unrelated_files() {
        let entries = fixture();
        let before = AgentPackage::from_manifest_entries(entries.clone(), "custom.toml").unwrap();
        let mut desired = before.clone();
        desired.manifest.instructions = "Updated instructions".into();
        let result =
            AgentPackage::edit_entries(entries.clone(), "custom.toml", &before, &desired).unwrap();
        assert_eq!(result["custom.toml"], entries["custom.toml"]);
        assert_eq!(result["README.md"], entries["README.md"]);
        assert_eq!(result["assets/data.bin"], entries["assets/data.bin"]);
        assert_eq!(result["prompt.md"], b"Updated instructions");
    }

    #[test]
    fn binary_edits_keep_remapped_source_paths_and_new_files_do_not_claim_repository_paths() {
        let entries = fixture();
        let before = AgentPackage::from_manifest_entries(entries.clone(), "custom.toml").unwrap();
        let mut desired = before.clone();
        let mut files = desired.files().unwrap();
        files[0].content = "AP4H".into();
        files.push(InitialFile {
            path: "/README.md".into(),
            content: "New runtime file".into(),
            encoding: "text".into(),
            is_readonly: true,
        });
        desired.manifest.initial_files = files.into_iter().map(File::Inline).collect();
        let result =
            AgentPackage::edit_entries(entries.clone(), "custom.toml", &before, &desired).unwrap();
        assert_eq!(result["assets/data.bin"], vec![0, 254, 7]);
        assert_eq!(result["README.md"], entries["README.md"]);
        let restored = AgentPackage::from_manifest_entries(result, "custom.toml").unwrap();
        assert!(restored.diff(&desired).unwrap().is_empty());
    }

    #[test]
    fn deleted_assets_are_not_rediscovered_and_unselected_files_survive() {
        let entries = fixture();
        let before = AgentPackage::from_manifest_entries(entries.clone(), "custom.toml").unwrap();
        let mut desired = before.clone();
        desired.manifest.initial_files.clear();
        let result =
            AgentPackage::edit_entries(entries.clone(), "custom.toml", &before, &desired).unwrap();
        assert!(!result.contains_key("assets/data.bin"));
        assert_eq!(result["README.md"], entries["README.md"]);
        assert!(
            AgentPackage::from_manifest_entries(result, "custom.toml")
                .unwrap()
                .files()
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn configuration_edits_keep_the_selected_manifest_format_and_instruction_reference() {
        for (name, text) in [
            (
                "agent.json",
                r#"{"schema_version":1,"name":"helper","instructions_file":"prompt.md"}"#,
            ),
            (
                "agent.yaml",
                "schema_version: 1\nname: helper\ninstructions_file: prompt.md\n",
            ),
            (
                "agent.md",
                "---\nschema_version: 1\nname: helper\ninstructions_file: prompt.md\n---\n",
            ),
        ] {
            let entries = [
                (name.into(), text.as_bytes().to_vec()),
                ("prompt.md".into(), b"Instructions".to_vec()),
            ]
            .into();
            let before = AgentPackage::from_manifest_entries(entries, name).unwrap();
            let mut desired = before.clone();
            desired.manifest.description = Some("Edited here".into());
            let entries = [
                (name.into(), text.as_bytes().to_vec()),
                ("prompt.md".into(), b"Instructions".to_vec()),
            ]
            .into();
            let result = AgentPackage::edit_entries(entries, name, &before, &desired).unwrap();
            let manifest = AgentPackage::parse(
                std::str::from_utf8(&result[name]).unwrap(),
                Format::from_extension(Path::new(name)),
            )
            .unwrap();
            assert_eq!(
                manifest.manifest.instructions_file.as_deref(),
                Some("prompt.md")
            );
            assert_eq!(
                manifest.manifest.description.as_deref(),
                Some("Edited here")
            );
        }
    }

    #[test]
    fn content_only_asset_edits_preserve_globs_and_manifest_comments() {
        let entries = fixture();
        let before = AgentPackage::from_manifest_entries(entries.clone(), "custom.toml").unwrap();
        let mut desired = before.clone();
        if let File::Inline(file) = &mut desired.manifest.initial_files[0] {
            file.content = "AP4H".into();
        }
        let result =
            AgentPackage::edit_entries(entries.clone(), "custom.toml", &before, &desired).unwrap();
        assert_eq!(result["custom.toml"], entries["custom.toml"]);
        assert_eq!(result["assets/data.bin"], vec![0, 254, 7]);
    }

    #[test]
    fn instruction_edits_cannot_silently_change_a_shared_runtime_asset() {
        let entries=[("agent.toml".into(),b"schema_version=1\nname='helper'\ninstructions_file='prompt.md'\nfiles=['prompt.md']\n".to_vec()),("prompt.md".into(),b"Old instructions".to_vec())].into();
        let before = AgentPackage::from_manifest_entries(entries, "agent.toml").unwrap();
        let mut desired = before.clone();
        desired.manifest.instructions = "New instructions".into();
        let entries=[("agent.toml".into(),b"schema_version=1\nname='helper'\ninstructions_file='prompt.md'\nfiles=['prompt.md']\n".to_vec()),("prompt.md".into(),b"Old instructions".to_vec())].into();
        assert!(AgentPackage::edit_entries(entries, "agent.toml", &before, &desired).is_err());
    }

    #[test]
    fn channel_config_edits_preserve_authored_enabled_intent_after_host_normalization() {
        let text=b"schema_version=1\nname='helper'\ninstructions='Help'\n[channels.chat]\ntype='ag_ui'\nenabled=true\n[channels.chat.config]\nrate_limit_per_minute=10\n".to_vec();
        let entries = [("agent.toml".into(), text)].into();
        let source = AgentPackage::from_manifest_entries(entries, "agent.toml").unwrap();
        let mut before = source.clone();
        before.manifest.channels.get_mut("chat").unwrap().enabled = false;
        let mut desired = before.clone();
        desired.manifest.channels.get_mut("chat").unwrap().config["rate_limit_per_minute"] =
            serde_json::json!(7);
        let entries = [(
            "agent.toml".into(),
            source.to_string(Format::Toml).unwrap().into_bytes(),
        )]
        .into();
        let result = AgentPackage::edit_entries(entries, "agent.toml", &before, &desired).unwrap();
        let result = AgentPackage::from_manifest_entries(result, "agent.toml").unwrap();
        assert!(result.manifest.channels["chat"].enabled);
        assert_eq!(
            result.manifest.channels["chat"].config["rate_limit_per_minute"],
            7
        );
    }

    #[test]
    fn manifest_edits_preserve_an_explicit_empty_file_selection() {
        let entries = [
            (
                "agent.toml".into(),
                b"schema_version=1\nname='helper'\ninstructions='Help'\nfiles=[]\n".to_vec(),
            ),
            (
                "files/unrelated.txt".into(),
                b"Destination must not adopt this".to_vec(),
            ),
        ]
        .into();
        let before = AgentPackage::from_manifest_entries(entries, "agent.toml").unwrap();
        let mut desired = before.clone();
        desired.manifest.description = Some("Changed description".into());
        let entries = [
            (
                "agent.toml".into(),
                b"schema_version=1\nname='helper'\ninstructions='Help'\nfiles=[]\n".to_vec(),
            ),
            (
                "files/unrelated.txt".into(),
                b"Destination must not adopt this".to_vec(),
            ),
        ]
        .into();
        let result = AgentPackage::edit_entries(entries, "agent.toml", &before, &desired).unwrap();
        assert!(
            AgentPackage::from_manifest_entries(result, "agent.toml")
                .unwrap()
                .files()
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn selecting_a_model_replaces_a_host_default_with_a_complete_authored_reference() {
        let text = b"schema_version=1\nname='helper'\ninstructions='Help'\n".to_vec();
        let entries = [("agent.toml".into(), text.clone())].into();
        let mut before = AgentPackage::from_manifest_entries(entries, "agent.toml").unwrap();
        before.manifest.model = Some(ModelSpec::on("openai", "host-default"));
        let mut desired = before.clone();
        desired.manifest.model = Some(ModelSpec::on("openai", "selected-model"));
        let entries = [("agent.toml".into(), text)].into();
        let result = AgentPackage::edit_entries(entries, "agent.toml", &before, &desired).unwrap();
        let result = AgentPackage::from_manifest_entries(result, "agent.toml").unwrap();
        assert_eq!(result.manifest.model.unwrap().model, "selected-model");
    }
}
