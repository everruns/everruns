use super::*;

async fn call(server: &TestServer, method: &str, params: Value) -> Value {
    mcp_call_with_headers(
        server,
        method,
        params,
        vec![("MCP-Protocol-Version", MCP_PROTOCOL_VERSION_2026)],
    )
    .await
}

#[tokio::test]
async fn public_skill_can_be_loaded_over_streamable_http() {
    let (_server, base_url) = TestServer::serving_in_memory().await;
    let client = reqwest::Client::new();
    for (method, mut params) in [
        (
            "skills/get",
            json!({"uri": "skill://everruns-mcp/SKILL.md"}),
        ),
        (
            "resources/read",
            json!({"uri": "skill://everruns-mcp/SKILL.md"}),
        ),
    ] {
        params["_meta"] = json!({
            "io.modelcontextprotocol/protocolVersion": MCP_PROTOCOL_VERSION_2026,
            "io.modelcontextprotocol/clientInfo": {"name": "skills-smoke", "version": "1"},
            "io.modelcontextprotocol/clientCapabilities": {},
        });
        let response: Value = client
            .post(format!("{base_url}/mcp"))
            .header("MCP-Protocol-Version", MCP_PROTOCOL_VERSION_2026)
            .header("Mcp-Method", method)
            .json(&json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}))
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json()
            .await
            .unwrap();
        assert!(response.get("error").is_none(), "{response}");
        assert_eq!(response["result"]["resultType"], "complete");
        if method == "skills/get" {
            assert_eq!(
                response["result"]["skill"]["frontmatter"]["name"],
                "everruns-mcp"
            );
        } else {
            assert!(
                response["result"]["contents"][0]["text"]
                    .as_str()
                    .unwrap()
                    .starts_with("---\n")
            );
        }
    }
}

#[tokio::test]
async fn mcp_skill_discovery_and_query_examples_work_on_the_endpoint() {
    let server = TestServer::in_memory().await;
    let discovered = mcp_tool_call(&server, "discover", json!({"query": "list_agents"})).await;
    assert!(!tool_is_error(&discovered), "{}", tool_text(&discovered));
    let payload = tool_json(&discovered);
    assert!(
        payload["operations"]
            .as_array()
            .unwrap()
            .iter()
            .any(|op| op["name"] == "list_agents")
    );
    let query = mcp_tool_call(
        &server,
        "query",
        json!({"commands": "everruns agents list"}),
    )
    .await;
    assert!(!tool_is_error(&query), "{}", tool_text(&query));
    assert!(tool_json(&query)["data"].is_array());
    let created = mcp_tool_call(&server, "execute", json!({
        "commands": "everruns agents create --name 'mcp-skill-demo' --system-prompt 'Answer concisely.' --reason 'Create the requested test agent'"
    })).await;
    assert!(!tool_is_error(&created), "{}", tool_text(&created));
    let stored = mcp_tool_call(
        &server,
        "query",
        json!({"commands": "everruns agents get mcp-skill-demo"}),
    )
    .await;
    assert!(!tool_is_error(&stored), "{}", tool_text(&stored));
    assert_eq!(tool_json(&stored)["name"], "mcp-skill-demo");
}

#[tokio::test]
async fn public_skills_round_trip_with_complete_verified_manifests() {
    let server = TestServer::in_memory().await;
    let discovery = call(&server, "server/discover", json!({})).await;
    assert_eq!(discovery["result"]["resultType"], "complete");
    assert_eq!(discovery["result"]["cacheScope"], "private");
    assert!(discovery["result"]["ttlMs"].as_u64().is_some());
    assert_eq!(
        discovery["result"]["capabilities"]["extensions"]["io.modelcontextprotocol/skills"]["directoryRead"],
        true
    );
    let list = call(&server, "skills/list", json!({})).await;
    assert_eq!(list["result"]["resultType"], "complete");
    assert_eq!(list["result"]["cacheScope"], "public");
    assert!(list["result"]["ttlMs"].as_u64().is_some());
    let skills = list["result"]["skills"].as_array().expect("skills");
    assert_eq!(skills.len(), 2);
    assert_eq!(
        skills
            .iter()
            .map(|s| s["frontmatter"]["name"].as_str().unwrap())
            .collect::<std::collections::BTreeSet<_>>(),
        std::collections::BTreeSet::from(["everruns-mcp", "everruns-agent-authoring"])
    );
    for skill in skills {
        let get = call(&server, "skills/get", json!({"uri": skill["uri"]})).await;
        assert_eq!(get["result"]["skill"], *skill);
        assert_eq!(get["result"]["cacheScope"], "public");
        let manifest = skill["resources"].as_array().expect("manifest");
        assert!(manifest.iter().any(|file| file["uri"] == skill["uri"]));
        for file in manifest {
            let read = call(&server, "resources/read", json!({"uri": file["uri"]})).await;
            assert_eq!(read["result"]["cacheScope"], "public");
            let contents = &read["result"]["contents"][0];
            assert_eq!(contents["uri"], file["uri"]);
            let bytes = if let Some(text) = contents["text"].as_str() {
                text.as_bytes().to_vec()
            } else {
                use base64::Engine;
                base64::engine::general_purpose::STANDARD
                    .decode(contents["blob"].as_str().expect("blob"))
                    .expect("base64")
            };
            assert_eq!(bytes.len() as u64, file["size"].as_u64().unwrap());
            assert_eq!(
                format!("sha256:{}", hex::encode(Sha256::digest(&bytes))),
                file["digest"]
            );
            if file["uri"] == skill["uri"] {
                let text = std::str::from_utf8(&bytes).unwrap();
                let frontmatter = text
                    .strip_prefix("---\n")
                    .unwrap()
                    .split_once("\n---")
                    .unwrap()
                    .0;
                let parsed: Value = serde_yaml::from_str(frontmatter).unwrap();
                assert_eq!(parsed, skill["frontmatter"]);
            }
        }
    }
}

#[tokio::test]
async fn skill_lookup_is_direct_and_rejects_unknown_or_traversing_uris() {
    let server = TestServer::in_memory().await;
    let get = call(
        &server,
        "skills/get",
        json!({"uri": "skill://everruns-mcp/SKILL.md"}),
    )
    .await;
    assert_eq!(
        get["result"]["skill"]["frontmatter"]["name"],
        "everruns-mcp"
    );
    for uri in [
        "skill://missing/SKILL.md",
        "skill://everruns/SKILL.md",
        "skill://everruns/references/crates.md",
        "skill://everruns",
        "skill://everruns/references",
        "skill://everruns-mcp/../everruns/SKILL.md",
        "skill://everruns-mcp/%2e%2e/everruns/SKILL.md",
        "file:///etc/passwd",
    ] {
        for method in ["skills/get", "resources/read", "resources/directory/read"] {
            let response = call(&server, method, json!({"uri": uri})).await;
            assert_eq!(response["error"]["code"], -32602, "{method} {uri}");
        }
    }
    for params in [json!({}), json!({"uri": 42})] {
        let response = call(&server, "skills/get", params).await;
        assert_eq!(response["error"]["code"], -32602);
    }
    let response = call(&server, "skills/list", json!({"cursor": "invalid"})).await;
    assert_eq!(response["error"]["code"], -32602);
    for uri in [
        "skill:/",
        "skill://everruns/",
        "skill://everruns/references/",
    ] {
        let response = call(&server, "resources/directory/read", json!({"uri": uri})).await;
        assert_eq!(response["error"]["code"], -32602);
    }
}

#[tokio::test]
async fn skill_directories_return_only_direct_children_and_are_2026_only() {
    let server = TestServer::in_memory().await;
    let result = call(
        &server,
        "resources/directory/read",
        json!({"uri": "skill://everruns-mcp"}),
    )
    .await;
    let children = result["result"]["resources"].as_array().unwrap();
    assert_eq!(children.len(), 1);
    assert_eq!(children[0]["name"], "SKILL.md");
    assert_eq!(children[0]["uri"], "skill://everruns-mcp/SKILL.md");
    for method in ["skills/list", "skills/get", "resources/directory/read"] {
        let legacy = mcp_call(
            &server,
            method,
            json!({"uri": "skill://everruns-mcp/SKILL.md"}),
        )
        .await;
        assert_eq!(legacy["error"]["code"], -32601);
    }
    let legacy = mcp_call(&server, "initialize", json!({})).await;
    assert!(legacy["result"]["capabilities"]["extensions"].is_null());
}
