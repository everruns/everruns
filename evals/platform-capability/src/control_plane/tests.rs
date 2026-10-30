use super::*;
use serde_json::{Value, json};

#[cfg(test)]
mod legacy {
    use super::*;

    fn plane() -> FakeControlPlane {
        FakeControlPlane::new(Harness::LegacyPlatformChat)
    }

    #[tokio::test]
    async fn a_tree_spelling_runs_and_pages_a_family() {
        let out = plane()
            .call("query", &json!({ "commands": "everruns agents list" }))
            .await;
        let value: Value = serde_json::from_str(&out).expect("json");
        assert_eq!(value["total"], 2, "{out}");
        assert_eq!(value["data"][0]["name"], "Triage");
    }

    /// The whole point of the fake: an absent flag is rejected by the real
    /// parser, so an offline case measures the same failure production has.
    #[tokio::test]
    async fn an_absent_flag_is_rejected_by_the_real_parser() {
        let out = plane()
            .call(
                "query",
                &json!({ "commands": "everruns skills list --limit 20" }),
            )
            .await;
        assert!(out.contains("unexpected argument"), "{out}");
        assert!(out.contains("--limit"), "{out}");
    }

    #[tokio::test]
    async fn a_flat_wire_name_still_works() {
        let out = plane()
            .call("query", &json!({ "commands": "list_harnesses" }))
            .await;
        assert!(out.contains("Generic"), "{out}");
    }

    #[tokio::test]
    async fn query_refuses_a_mutation_and_execute_runs_it() {
        let refused = plane()
            .call(
                "query",
                &json!({ "commands": "everruns agents create --name Bot --system-prompt Hi --reason Requested" }),
            )
            .await;
        assert!(refused.contains("use execute"), "{refused}");

        let plane = plane();
        let created = plane
            .call(
                "execute",
                &json!({ "commands": "everruns agents create --name Bot --system-prompt Hi --reason Requested" }),
            )
            .await;
        assert!(created.contains("\"name\":\"Bot\""), "{created}");
        // And it persists for the rest of the turn, so a follow-up read works.
        let listed = plane
            .call("query", &json!({ "commands": "everruns agents list" }))
            .await;
        assert!(listed.contains("Bot"), "{listed}");
    }

    #[tokio::test]
    async fn a_bare_word_reaches_the_positional_field() {
        let out = plane()
            .call(
                "query",
                &json!({ "commands": "everruns agents get agent_01triage" }),
            )
            .await;
        assert!(out.contains("Triage"), "{out}");
    }

    #[tokio::test]
    async fn discover_finds_an_operation_and_carries_its_spelling() {
        let out = plane()
            .call("discover", &json!({ "query": "list_harnesses" }))
            .await;
        assert!(out.contains("list_harnesses"), "{out}");
        assert!(
            out.contains("input_schema"),
            "an exact lookup carries schemas: {out}"
        );
    }

    /// A broad search must not expand every schema, which is the rule the real
    /// surface follows to keep discovery affordable.
    #[tokio::test]
    async fn a_broad_search_omits_schemas() {
        let out = plane().call("discover", &json!({ "query": "agent" })).await;
        assert!(!out.contains("input_schema"), "{out}");
    }

    /// The help is the tree's, not a local restatement: a hand-rolled list of
    /// nouns without descriptions is a different surface from the one that
    /// ships, and in the shell surface `--help` is nearly the whole discovery story.
    #[tokio::test]
    async fn help_is_the_shipped_rendering() {
        let root = plane()
            .call("query", &json!({ "commands": "everruns --help" }))
            .await;
        assert!(
            root.contains("Agent definitions and their configuration."),
            "node descriptions travel with the help: {root}"
        );
        let node = plane()
            .call("query", &json!({ "commands": "everruns agents --help" }))
            .await;
        assert!(
            node.contains("Run `everruns agents <command> --help` for flags."),
            "{node}"
        );
        assert!(!node.contains("unknown command"), "{node}");
        for probe in ["everruns agents triggers --help", "everruns agents -h"] {
            let out = plane().call("query", &json!({ "commands": probe })).await;
            assert!(
                !out.contains("unknown command") && out.contains("Usage:"),
                "{probe}: {out}"
            );
        }
        let unknown = plane()
            .call(
                "query",
                &json!({ "commands": "everruns agents frobnicate" }),
            )
            .await;
        assert!(
            unknown.contains("unknown command `frobnicate`"),
            "{unknown}"
        );
    }

    #[tokio::test]
    async fn help_lists_the_nouns_and_a_nodes_verbs() {
        let root = plane()
            .call("query", &json!({ "commands": "everruns --help" }))
            .await;
        assert!(root.contains("agents"), "{root}");
        let node = plane()
            .call("query", &json!({ "commands": "everruns agents" }))
            .await;
        assert!(node.contains("triggers") && node.contains("list"), "{node}");
    }

    #[tokio::test]
    async fn a_pipeline_runs_its_command_and_ignores_the_filter() {
        let out = plane()
            .call(
                "query",
                &json!({ "commands": "everruns agents list | jq -r '.data[].id'" }),
            )
            .await;
        assert!(out.contains("agent_01triage"), "{out}");
    }

    #[tokio::test]
    async fn several_statements_run_in_order() {
        let plane = plane();
        let out = plane.call(
            "execute",
            &json!({ "commands": "everruns agents create --name A --system-prompt X --reason Requested\neverruns agents list" }),
        ).await;
        assert!(out.contains("\"name\":\"A\""), "{out}");
        assert!(out.matches("Triage").count() >= 1, "{out}");
    }
}

/// The canonical arm: one `bash` tool over a real interpreter.
///
/// These are the cases the legacy surface's statement splitter got wrong or could not express
/// at all. They are the reason the A/B is worth running: if the shell arm can
/// loop, pipe and keep a file, its failures are the model's and the contract's
/// rather than the fake's.
#[cfg(test)]
mod shell_tests {
    use super::*;

    fn shell() -> FakeControlPlane {
        FakeControlPlane::new(Harness::PlatformChat)
    }

    #[test]
    fn a_help_probe_is_not_a_mutation() {
        assert!(script_mutates("everruns agents create --name Bot"));
        assert!(!script_mutates("everruns agents create --help"));
        assert!(!script_mutates("everruns agents list | jq '.data'"));
        // A script that reads and then writes is still a write.
        assert!(script_mutates(
            "everruns agents create --help\neveruns agents list\neverruns agents update --id a"
        ));
    }

    #[tokio::test]
    async fn canonical_surface_ships_one_tool_and_it_is_bash() {
        let tools = tool_definitions(Harness::PlatformChat);
        let names: Vec<&str> = tools
            .iter()
            .filter_map(|t| t["function"]["name"].as_str())
            .collect();
        assert_eq!(names, vec!["bash"]);
        // And the prompt it ships with does not send the model after tools it
        // no longer has.
        let prompt = system_prompt(Harness::PlatformChat);
        assert!(!prompt.contains("`discover`"), "{prompt}");
        assert!(!prompt.contains("`query`"), "{prompt}");
        assert!(!prompt.contains("`execute`"), "{prompt}");
    }

    #[tokio::test]
    async fn everruns_is_a_builtin_in_the_shell() {
        let out = shell()
            .call("bash", &json!({ "commands": "everruns agents list" }))
            .await;
        assert!(out.contains("Triage"), "{out}");
    }

    /// the legacy surface's splitter truncated at the first `|` and returned the command's own
    /// JSON. Here the filter runs.
    #[tokio::test]
    async fn a_pipeline_runs_its_filter() {
        let out = shell()
            .call(
                "bash",
                &json!({ "commands": "everruns agents list | jq -r '.data[].name'" }),
            )
            .await;
        assert!(out.contains("Triage"), "{out}");
        assert!(
            !out.contains("\"id\""),
            "jq projected, not passed through: {out}"
        );
    }

    /// The defect that made the legacy surface's arm unfair: `for … do … done` was split into
    /// words and the loop body never ran as a loop.
    #[tokio::test]
    async fn a_loop_runs_as_a_loop() {
        let out = shell()
            .call(
                "bash",
                &json!({ "commands": "for name in Alpha Beta; do everruns agents create --name \"$name\" --system-prompt Hi --reason Requested > /dev/null; done\neveruns_count=$(everruns agents list | jq '.total')\necho \"total=$everuns_count\"" }),
            )
            .await;
        assert!(
            out.contains("total=4"),
            "two seeded plus two created: {out}"
        );
    }

    /// Redirection and a scratch file, which the legacy surface has no filesystem for.
    #[tokio::test]
    async fn output_survives_in_the_workspace_between_calls() {
        let plane = shell();
        plane
            .call(
                "bash",
                &json!({ "commands": "everruns harnesses list > /workspace/harnesses.json" }),
            )
            .await;
        let out = plane
            .call(
                "bash",
                &json!({ "commands": "jq -r '.data[].name' /workspace/harnesses.json" }),
            )
            .await;
        assert!(out.contains("Generic"), "{out}");
    }

    #[tokio::test]
    async fn help_reaches_the_shell_and_a_bad_flag_fails_nonzero() {
        let out = shell()
            .call("bash", &json!({ "commands": "everruns --help" }))
            .await;
        assert!(out.contains("agents"), "{out}");

        let bad = shell()
            .call(
                "bash",
                &json!({ "commands": "everruns skills list --limit 20" }),
            )
            .await;
        assert!(bad.contains("unexpected argument"), "{bad}");
        assert!(
            bad.contains("[exit "),
            "a parse error is a failed command: {bad}"
        );
    }

    /// The shell surface has no read-only gate: a mutation runs, because the
    /// split into `query` and `execute` was a property of having two tools.
    #[tokio::test]
    async fn a_mutation_runs_without_a_second_tool() {
        let out = shell()
            .call(
                "bash",
                &json!({ "commands": "everruns agents create --name Bot --system-prompt Hi --reason Requested" }),
            )
            .await;
        assert!(out.contains("\"name\":\"Bot\""), "{out}");
    }

    #[tokio::test]
    async fn the_docs_mount_is_readable() {
        let out = shell()
            .call(
                "bash",
                &json!({ "commands": "grep -r Generic /workspace/docs" }),
            )
            .await;
        assert!(out.contains("Generic"), "{out}");
    }
}

#[cfg(test)]
mod family_tests {
    use super::*;

    fn family(path: &str, name: &str) -> String {
        family_of(name, &json!({ "path": path }))
    }

    /// A nested collection is its own family. Filing `create_agent_trigger`
    /// under `agents` made `list_agent_triggers` return agents, which is worse
    /// than an error: the model believes it and keeps going.
    #[tokio::test]
    async fn a_nested_collection_is_its_own_family() {
        assert_eq!(
            family("/v1/agents/{agent_id}/triggers", "create_agent_trigger"),
            "triggers"
        );
        assert_eq!(family("/v1/agents", "list_agents"), "agents");
        assert_eq!(family("/v1/agents/{id}", "get_agent"), "agents");
        assert_eq!(
            family("/v1/sessions/{session_id}/messages", "create_message"),
            "messages"
        );
    }

    #[tokio::test]
    async fn a_created_trigger_is_listed_by_its_own_command() {
        let plane = FakeControlPlane::new(Harness::LegacyPlatformChat);
        plane.call(
            "execute",
            &json!({ "commands": "create_agent_trigger --agent_id agent_01triage --message before --reason Requested" }),
        ).await;
        let listed = plane
            .call(
                "query",
                &json!({ "commands": "list_agent_triggers --agent_id agent_01triage" }),
            )
            .await;
        assert!(listed.contains("before"), "{listed}");
        // And the agents family is untouched.
        let agents = plane
            .call("query", &json!({ "commands": "everruns agents list" }))
            .await;
        let value: Value = serde_json::from_str(&agents).expect("json");
        assert_eq!(value["total"], 2, "{agents}");
    }
}

#[cfg(test)]
mod native_tests {
    use super::*;

    #[test]
    fn eager_sends_every_authentic_operation_and_full_schema() {
        let tools = tool_definitions(Harness::NativeEager);
        let entries = catalog();
        assert!(entries.len() >= 292);
        assert_eq!(tools.len(), entries.len());
        for (tool, entry) in tools.iter().zip(entries.iter()) {
            assert_eq!(tool["function"]["name"], entry["name"]);
            assert_eq!(tool["function"]["description"], entry["description"]);
            assert_eq!(
                tool["function"]["parameters"],
                native_schema(&entry["input_schema"])
            );
        }
    }

    #[test]
    fn captured_schemas_validate_without_external_references() {
        fn local_refs(value: &Value) {
            match value {
                Value::Object(fields) => {
                    if let Some(reference) = fields.get("$ref").and_then(Value::as_str) {
                        assert!(
                            reference.starts_with('#'),
                            "nonlocal catalog reference: {reference}"
                        );
                    }
                    for child in fields.values() {
                        local_refs(child);
                    }
                }
                Value::Array(items) => {
                    for item in items {
                        local_refs(item);
                    }
                }
                _ => {}
            }
        }
        for entry in catalog() {
            local_refs(&entry["input_schema"]);
            assert!(
                jsonschema::validator_for(&entry["input_schema"]).is_ok(),
                "invalid schema: {}",
                entry["name"]
            );
            assert!(
                jsonschema::validator_for(&native_schema(&entry["input_schema"])).is_ok(),
                "invalid enveloped schema: {}",
                entry["name"]
            );
        }
    }

    #[tokio::test]
    async fn native_and_cli_share_validated_dispatch_and_state() {
        let native = FakeControlPlane::new(Harness::NativeEager);
        let cli = FakeControlPlane::new(Harness::PlatformChat);
        let docs_context: Value = serde_json::from_str(
            &native
                .call(
                    "get_manager_context",
                    &json!({"params":{"entity_ref":"agent_01docs"}}),
                )
                .await,
        )
        .unwrap();
        assert_eq!(docs_context["content"], "");
        assert_eq!(docs_context["revision"], 0);
        // Compare identical dispatch histories after the context fixture check.
        cli.call(
            "bash",
            &json!({"commands":"everruns context get agent_01docs"}),
        )
        .await;
        let args = json!({"params":{"name":"surface-helper","system_prompt":"Answer briefly."},"reason":"Requested"});
        let result = native.call("create_agent", &args).await;
        assert!(result.contains("surface-helper"), "{result}");
        cli.call("bash", &json!({"commands":"everruns agents create --name surface-helper --system-prompt 'Answer briefly.' --reason Requested"})).await;
        assert_eq!(native.snapshot(), cli.snapshot());
        assert!(
            native
                .call("list_agents", &json!({"params":{}}))
                .await
                .contains("surface-helper")
        );
        let before = native.snapshot()["families"].clone();
        let invalid = native
            .call(
                "create_agent",
                &json!({"params":{"name":42},"reason":"Requested"}),
            )
            .await;
        assert!(invalid.contains("invalid arguments"), "{invalid}");
        assert_eq!(
            native.snapshot()["families"],
            before,
            "invalid JSON cannot mutate state"
        );
    }
}
