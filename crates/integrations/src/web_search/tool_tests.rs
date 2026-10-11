use super::*;
use crate::web_search::free::FreeSearchEndpoints;
use everruns_contracts::runtime::tools::ToolExecutionResult;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const DDG_HTML: &str = include_str!("fixtures/duckduckgo_html.html");
const DDG_BOT_CHECK: &str = include_str!("fixtures/duckduckgo_bot_check.html");

async fn server(html_status: u16, html: &str) -> MockServer {
    let server = MockServer::start().await;
    for (at, status, body) in [
        ("/html/", html_status, html),
        ("/lite/", 202, DDG_BOT_CHECK),
        ("/mojeek", 403, "automated queries"),
        ("/w/api.php", 503, ""),
        ("/ia", 200, "{}"),
    ] {
        Mock::given(method("GET"))
            .and(path(at))
            .respond_with(ResponseTemplate::new(status).set_body_string(body))
            .mount(&server)
            .await;
    }
    server
}

fn tool(server: &MockServer, config: Value) -> WebSearchTool {
    let base = server.uri();
    WebSearchTool::new(WebSearchConfig::from_value(&config).unwrap(), None).with_free(
        FreeSearchDriver::with_endpoints(FreeSearchEndpoints {
            duckduckgo_html: format!("{base}/html/"),
            duckduckgo_lite: format!("{base}/lite/"),
            mojeek: format!("{base}/mojeek"),
            wikipedia: format!("{base}/w/api.php"),
            instant_answer: format!("{base}/ia"),
            github: format!("{base}/github"),
            npm: format!("{base}/npm"),
            crates: format!("{base}/crates"),
        }),
    )
}

fn split(result: ToolExecutionResult) -> (Value, Value) {
    let result = result.into_tool_result("call_1", "web_search");
    (result.result.unwrap(), result.details.unwrap())
}

#[tokio::test]
async fn results_are_citable_and_the_provider_stays_in_the_details() {
    let server = server(200, DDG_HTML).await;
    let (model, details) = split(
        tool(&server, json!({}))
            .execute(json!({"query": "tokio rust", "limit": 2}))
            .await,
    );

    let results = model["results"].as_array().unwrap();
    assert_eq!(results.len(), 2);
    assert_eq!(results[0]["url"], "https://tokio.rs/");
    assert_eq!(results[0]["site"], "tokio.rs");
    assert_eq!(results[0]["id"], result_id("https://tokio.rs/"));
    assert_eq!(results[0]["published"], "2026-10-02");
    // show_provider is off: the model does not see who answered.
    assert!(model.get("provider").is_none());

    assert_eq!(details["provider"], "free");
    assert_eq!(details["provider_label"], "Free search");
    assert_eq!(details["setting"]["provider"], "org_default");
    assert_eq!(details["plan"], json!(["free"]));
    let exchanges = details["attempts"][0]["exchanges"].as_array().unwrap();
    assert!(exchanges[0]["body"].as_str().unwrap().contains("result__a"));
}

#[tokio::test]
async fn show_provider_tells_the_model_who_answered_and_why() {
    let server = server(200, DDG_HTML).await;
    let (model, details) = split(
        tool(&server, json!({"provider": "brave", "show_provider": true}))
            .execute(json!({"query": "tokio"}))
            .await,
    );
    assert_eq!(model["provider"], "Free search");
    assert_eq!(
        model["notes"][0],
        "Brave Search: No Brave Search key is connected; answered by Free search."
    );
    assert_eq!(
        narrate_outcome(&details, None).unwrap(),
        "Found 3 results with Free search (Brave Search is not connected)"
    );
}

#[tokio::test]
async fn total_failure_keeps_the_raw_responses_and_says_why() {
    let server = server(202, DDG_BOT_CHECK).await;
    let (model, details) = split(
        tool(&server, json!({"provider": "free"}))
            .execute(json!({"query": "tokio"}))
            .await,
    );
    assert!(
        model["error"]
            .as_str()
            .unwrap()
            .contains("DuckDuckGo showed a bot check")
    );
    assert_eq!(details["ok"], false);
    assert_eq!(
        details["attempts"][0]["exchanges"].as_array().unwrap().len(),
        5
    );
    assert!(
        narrate_outcome(&details, None)
            .unwrap()
            .starts_with("Web search failed: Free search could not reach a search engine")
    );
}

#[tokio::test]
async fn agent_domain_filters_cannot_be_widened_by_the_model() {
    let server = server(200, DDG_HTML).await;
    let tool = tool(&server, json!({"blocked_domains": ["github.com"]}));
    let request = tool
        .request(Arguments {
            query: "tokio".into(),
            limit: Some(50),
            freshness: None,
            domains: vec!["github.com".into(), "docs.rs".into()],
            topic: SearchTopic::General,
        })
        .unwrap();
    assert_eq!(request.allowed_domains, ["docs.rs"]);
    assert_eq!(request.blocked_domains, ["github.com"]);
    assert_eq!(request.limit, WEB_SEARCH_MAX_RESULTS);
}

#[test]
fn rejects_an_empty_query_and_unknown_arguments() {
    let tool = WebSearchTool::new(WebSearchConfig::default(), None);
    assert!(
        parse(json!({"query": "  "}))
            .and_then(|arguments| tool.request(arguments))
            .is_err()
    );
    assert!(parse(json!({"query": "x", "engine": "bing"})).is_err());
}

#[test]
fn result_ids_are_stable_per_url() {
    assert_eq!(result_id("https://tokio.rs/"), result_id("https://tokio.rs"));
    assert_ne!(result_id("https://tokio.rs/"), result_id("https://docs.rs/"));
    assert_eq!(result_id("https://tokio.rs/").len(), 7);
}

#[test]
fn narration_names_sites_and_handles_empty_results() {
    let details = json!({
        "query": "tokio", "ok": true, "provider_label": "Free search",
        "attempts": [{"ok": true, "label": "Free search"}],
        "results": [
            {"site": "tokio.rs"}, {"site": "github.com"}, {"site": "tokio.rs"}, {"site": "docs.rs"}
        ]
    });
    assert_eq!(
        narrate_outcome(&details, None).unwrap(),
        "Found 4 results on tokio.rs, github.com and docs.rs"
    );
    assert_eq!(
        narrate_outcome(&details, Some("uk")).unwrap(),
        "Знайдено результатів: 4 на tokio.rs, github.com і docs.rs"
    );
    let empty = json!({"query": "zzz", "ok": true, "attempts": [], "results": []});
    assert_eq!(narrate_outcome(&empty, None).unwrap(), "No results for \"zzz\"");
}
