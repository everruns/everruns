use super::*;
use everruns_contracts::ToolDefinition;

#[test]
fn active_needs_the_option_the_tool_and_a_toolset_model() {
    let mut config = LlmCallConfig::new("claude-opus-5-5");
    config.tools = vec![ToolDefinition::function("browser", "", json!({}))];
    assert!(!active(&config, "claude-opus-5-5"));
    let (key, value) = NativeBrowserUse::default().to_driver_option();
    config.driver_options.insert(key, value);
    assert!(active(&config, "claude-opus-5-5"));
    assert!(!active(&config, "claude-haiku-4-5"));
}

#[test]
fn entry_keeps_the_default_members() {
    assert_eq!(toolset_entry(false), json!({ "type": TOOLSET_TYPE }));
    assert_eq!(toolset_entry(true)["cache_control"]["type"], "ephemeral");
}

#[test]
fn member_calls_become_browser_calls_batched_per_toolset() {
    let computer = ToolCall {
        id: "toolu_0".into(),
        name: "computer".into(),
        arguments: json!({ "action": "screenshot", "native_batch": { "id": "toolu_0" } }),
    };
    let mut first = ToolCall {
        id: "toolu_1".into(),
        name: "find".into(),
        arguments: json!({ "query": "search box" }),
    };
    into_browser_call(&mut first, std::slice::from_ref(&computer));
    assert_eq!(first.name, "browser");
    // The computer call's batch is not the browser's.
    assert_eq!(
        first.arguments,
        json!({ "action": "find", "query": "search box", "native_batch": { "id": "toolu_1" } })
    );
    let mut second = ToolCall {
        id: "toolu_2".into(),
        name: "new_tab".into(),
        arguments: json!(""),
    };
    into_browser_call(&mut second, &[computer, first]);
    assert_eq!(
        second.arguments,
        json!({ "action": "new_tab", "native_batch": { "id": "toolu_1" } })
    );
}

fn state() -> Value {
    json!({
        "tabs": [
            { "tab_id": "T1", "title": "Docs", "url": "https://example.com/docs", "active": true },
            { "tab_id": "T2", "title": "", "url": "about:blank" }
        ],
        "state_changes": [{ "type": "tab_opened", "tab_id": "T2" }]
    })
}

#[test]
fn replay_rebuilds_results_in_the_toolset_shape() {
    let read = json!({ "status": "ok", "action": "read_page", "text": "button \"Go\" [ref_1]",
                       "browser_state": { "tabs": state()["tabs"] } });
    let shot = json!({ "status": "ok", "action": "screenshot", "text": "",
                       "browser_state": state() });
    let tab = json!({ "status": "ok", "action": "new_tab",
                      "text": "Created new tab with tab_id: T2, URL: about:blank. It is now the current tab.",
                      "browser_state": state() });
    let image = json!({ "type": "image",
                        "source": { "type": "base64", "media_type": "image/png", "data": "AAAA" } });
    let mut messages = vec![
        json!({ "role": "assistant", "content": [
            { "type": "tool_use", "id": "b1", "name": "browser",
              "input": { "action": "read_page", "filter": "interactive", "native_batch": { "id": "b1" } } },
            { "type": "tool_use", "id": "b2", "name": "screenshot", "toolset_name": "browser", "input": {} },
            { "type": "tool_use", "id": "b3", "name": "browser", "input": { "action": "new_tab" } },
            { "type": "tool_use", "id": "b4", "name": "browser",
              "input": { "action": "left_click", "target": { "type": "ref", "ref": "ref_9" } } },
            { "type": "tool_use", "id": "w1", "name": "web_fetch", "input": {} },
            { "type": "tool_use", "id": "b5", "name": "browser", "input": { "action": "javascript_exec" } },
        ]}),
        json!({ "role": "user", "content": [
            { "type": "tool_result", "tool_use_id": "b1", "content": read.to_string() },
            { "type": "tool_result", "tool_use_id": "b2",
              "content": [{ "type": "text", "text": shot.to_string() }, image.clone()] },
            { "type": "tool_result", "tool_use_id": "b3", "content": tab.to_string() },
            { "type": "tool_result", "tool_use_id": "b4", "is_error": true,
              "content": "ref_9 is stale or not found on the current page. Re-read the page to get fresh references." },
            { "type": "tool_result", "tool_use_id": "w1", "content": "{}" },
            { "type": "tool_result", "tool_use_id": "b5", "is_error": true, "content": "no" },
        ]}),
    ];
    rewrite_messages(&mut messages);

    let calls = messages[0]["content"].as_array().unwrap();
    assert_eq!(
        calls[0],
        json!({ "type": "tool_use", "id": "b1", "name": "read_page", "toolset_name": "browser",
                "input": { "filter": "interactive" } })
    );
    assert_eq!(calls[2]["name"], "new_tab");
    assert_eq!(calls[3]["name"], "left_click");
    assert!(calls[4].get("toolset_name").is_none());
    // Not a member: left as a plain `browser` call.
    assert_eq!(calls[5]["name"], "browser");

    let results = messages[1]["content"].as_array().unwrap();
    assert_eq!(
        results[0]["content"],
        json!([
            { "type": "text", "text": "button \"Go\" [ref_1]" },
            { "type": "browser_state", "tabs": state()["tabs"] }
        ])
    );
    assert_eq!(results[0]["toolset_name"], "browser");
    // A screenshot: placeholder text, the image, then the state with changes.
    let shot_content = results[1]["content"].as_array().unwrap();
    assert_eq!(shot_content[0], json!({ "type": "text", "text": "Done." }));
    assert_eq!(shot_content[1], image);
    assert_eq!(
        shot_content[2]["state_changes"],
        json!([{ "type": "tab_opened", "tab_id": "T2" }])
    );
    // A tab member: the state block alone.
    let tab_content = results[2]["content"].as_array().unwrap();
    assert_eq!(tab_content.len(), 1);
    assert_eq!(tab_content[0]["type"], "browser_state");
    // An error keeps its text and gets no state.
    assert_eq!(results[3]["toolset_name"], "browser");
    assert!(results[3]["content"].is_string());
    assert!(results[4].get("toolset_name").is_none());
    assert!(results[5].get("toolset_name").is_none());
}
