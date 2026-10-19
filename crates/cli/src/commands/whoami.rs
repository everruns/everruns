// CLI whoami command: who the server says the credential belongs to.
//
// Decision: ask `GET /v1/auth/me` rather than read the credential file, so the
// answer covers `--api-key` and `EVERRUNS_API_KEY` too and reflects what the
// server enforces. `status` stays the offline view of the stored profile.
// Decision: the access line comes from `token_scope`, which the server sets
// only for a personal access token (knowledge/security/pat-scopes.md,
// Decision 5). A cookie or JWT has no scope, so nothing extra is printed.
// JSON and YAML print the server's document unchanged, `token_scope` included.

use anyhow::Result;
use serde_json::Value;

use super::api::ApiClient;
use crate::output::OutputFormat;

pub async fn run(client: &ApiClient<'_>, output: OutputFormat) -> Result<()> {
    let me = client.get("/v1/auth/me").await?;
    if output.is_text() {
        for line in text(&me) {
            println!("{line}");
        }
    } else {
        output.print_value(&me);
    }
    Ok(())
}

/// How a token scope reads to a person. An unknown scope is shown as sent.
pub fn access_label(scope: &str) -> &str {
    match scope {
        "*" => "full",
        "read" => "read-only",
        other => other,
    }
}

fn text(me: &Value) -> Vec<String> {
    let field = |name: &str| me.get(name).and_then(Value::as_str).unwrap_or_default();
    let mut lines = vec![format!("User:    {} ({})", field("name"), field("email"))];
    if let Some(scope) = me.get("token_scope").and_then(Value::as_str) {
        lines.push(format!(
            "Access:  {} (personal access token)",
            access_label(scope)
        ));
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn me(token_scope: Value) -> Value {
        json!({
            "id": "user_1",
            "email": "ada@example.com",
            "name": "Ada",
            "token_scope": token_scope,
        })
    }

    #[test]
    fn a_read_token_shows_read_only_access() {
        assert_eq!(
            text(&me(json!("read"))),
            [
                "User:    Ada (ada@example.com)",
                "Access:  read-only (personal access token)",
            ]
        );
    }

    #[test]
    fn a_full_token_shows_full_access() {
        assert_eq!(
            text(&me(json!("*")))[1],
            "Access:  full (personal access token)"
        );
    }

    #[test]
    fn a_session_without_a_token_scope_shows_no_access_line() {
        assert_eq!(text(&me(Value::Null)), ["User:    Ada (ada@example.com)"]);
        let mut without_field = me(Value::Null);
        without_field.as_object_mut().unwrap().remove("token_scope");
        assert_eq!(text(&without_field).len(), 1);
    }

    #[tokio::test]
    async fn asks_the_server_with_the_credential() {
        let (url, captured) = crate::commands::api::capture::serve_once(
            200,
            r#"{"id":"user_1","email":"ada@example.com","name":"Ada","token_scope":"read"}"#,
        )
        .await;
        let client = ApiClient::new(&url, "evr_pat_read", None);
        run(&client, OutputFormat::Json).await.unwrap();
        let request = captured.await.unwrap();
        assert!(request.headers[0].starts_with("GET /v1/auth/me "));
        assert_eq!(
            request.header("authorization").as_deref(),
            Some("Bearer evr_pat_read")
        );
    }
}
