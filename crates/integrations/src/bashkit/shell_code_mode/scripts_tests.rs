//! Saved scripts run with the caller's tools and the same stop rules.

use super::*;
use everruns_contracts::runtime::saved_scripts::{SavedScript, SavedScriptStore, SavedScripts};

#[derive(Default)]
struct Scripts(Mutex<Vec<SavedScript>>);

#[async_trait]
impl SavedScriptStore for Scripts {
    async fn list(&self) -> Result<Vec<SavedScript>, String> {
        Ok(self.0.lock().unwrap().clone())
    }
    async fn save(&self, script: SavedScript) -> Result<SavedScript, String> {
        let mut all = self.0.lock().unwrap();
        all.retain(|s| s.name != script.name);
        all.push(script.clone());
        Ok(script)
    }
}

fn script(name: &str, body: &str) -> SavedScript {
    SavedScript {
        name: name.to_string(),
        description: format!("The {name} script."),
        input_schema: None,
        body: body.to_string(),
    }
}

fn with_scripts(
    policy: Policy,
    scripts: Vec<SavedScript>,
    can_save: bool,
) -> (ToolContext, Arc<Policy>, Arc<Scripts>) {
    let policy = Arc::new(policy);
    let store = Arc::new(Scripts(Mutex::new(scripts)));
    let context = context(Some(policy.clone()), true).with_extension(Arc::new(SavedScripts {
        store: store.clone(),
        can_save,
    }));
    (context, policy, store)
}

#[tokio::test]
async fn a_saved_script_reads_its_input_and_calls_tools() {
    let mut fetch = script(
        "fetch-repo",
        r#"repo=$(jq -r .repo); tools web-fetch repo="$repo" | jq -c .input"#,
    );
    fetch.input_schema = Some(json!({
        "type": "object",
        "properties": {"repo": {"type": "string"}},
        "required": ["repo"]
    }));
    let (context, policy, _) = with_scripts(Policy::default(), vec![fetch], false);

    let output = run("tools scripts fetch-repo repo=a/b", &context).await;
    assert_eq!(output["stdout"], "{\"repo\":\"a/b\"}\n", "{output}");
    assert_eq!(policy.after.lock().unwrap().as_slice(), ["web_fetch"]);

    let listed = run("tools scripts", &context).await;
    assert!(
        listed["stdout"]
            .as_str()
            .unwrap()
            .contains("fetch-repo  The fetch-repo script."),
        "{listed}"
    );
    let bad = run("tools scripts fetch-repo", &context).await;
    assert_eq!(
        error_code(&bad),
        "invalid_input",
        "the schema is checked: {bad}"
    );
    let unknown = run("tools scripts nope", &context).await;
    assert_eq!(error_code(&unknown), "unknown_command");
}

#[tokio::test]
async fn a_stop_inside_a_saved_script_stops_its_caller() {
    let gated = script("gated", "tools github get-issue number=7; echo inner-after");
    let (context, policy, _) = with_scripts(
        Policy {
            approval_for: Some("mcp_github__get_issue"),
            ..Policy::default()
        },
        vec![gated],
        false,
    );
    let output = run("tools scripts gated; echo outer-after", &context).await;
    let stdout = output["stdout"].as_str().unwrap();
    assert!(!stdout.contains("after"), "{output}");
    assert_eq!(output["tools"]["stopped"]["reason"], "needs_approval");
    assert!(policy.after.lock().unwrap().is_empty());
}

#[tokio::test]
async fn saving_needs_manage_scripts_and_a_body() {
    let (context, _, store) = with_scripts(Policy::default(), vec![], false);
    let denied = run(
        "echo 'echo hi' | tools scripts save hello --description 'Say hi.'",
        &context,
    )
    .await;
    assert_eq!(error_code(&denied), "denied", "{denied}");
    assert!(store.0.lock().unwrap().is_empty());

    let (context, _, store) = with_scripts(Policy::default(), vec![], true);
    let empty = run("tools scripts save hello --description 'Say hi.'", &context).await;
    assert_eq!(error_code(&empty), "invalid_input", "{empty}");
    let bad_name = run(
        "echo 'echo hi' | tools scripts save Hello --description 'Say hi.'",
        &context,
    )
    .await;
    assert_eq!(error_code(&bad_name), "invalid_input");

    let saved = run(
        "echo 'echo hi from script' | tools scripts save hello --description 'Say hi.' && tools scripts hello",
        &context,
    )
    .await;
    assert_eq!(saved["exit_code"], 0, "{saved}");
    assert!(
        saved["stdout"]
            .as_str()
            .unwrap()
            .ends_with("hi from script\n"),
        "{saved}"
    );
    assert_eq!(store.0.lock().unwrap()[0].description, "Say hi.");
}

#[tokio::test]
async fn saved_scripts_nest_only_so_deep() {
    let (context, _, _) = with_scripts(
        Policy::default(),
        vec![script("loop", "tools scripts loop")],
        false,
    );
    let output = run("tools scripts loop", &context).await;
    assert_ne!(output["exit_code"], 0);
    assert!(
        output["stderr"]
            .as_str()
            .unwrap()
            .contains("nest more than"),
        "{output}"
    );
}

#[tokio::test]
async fn without_a_store_scripts_is_an_ordinary_word() {
    let context = context(Some(Arc::new(Policy::default())), true);
    let output = run("tools scripts", &context).await;
    assert_eq!(error_code(&output), "unknown_command", "{output}");
}
