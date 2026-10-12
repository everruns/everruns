//! System egress policy pre-checks on the fetch path.

use super::*;
use everruns_contracts::runtime::{EgressPolicyMode, SystemEgressPolicy};

#[test]
fn curated_writes_lets_fetches_through_and_names_denials() {
    let tool = WebFetchTool {
        system_policy: Some(Arc::new(SystemEgressPolicy::embedded(
            EgressPolicyMode::CuratedWrites,
        ))),
        ..Default::default()
    };
    assert!(
        tool.system_policy_block("https://blog.example.net/post")
            .is_none()
    );
    for (url, needle) in [
        ("https://webhook.site/abc", "request-capture"),
        ("http://203.0.113.9/", "hostname"),
    ] {
        match tool.system_policy_block(url) {
            Some(ToolExecutionResult::ToolError(message)) => {
                assert!(message.contains(needle), "{url}: {message}")
            }
            other => panic!("{url} should be blocked, got {other:?}"),
        }
    }
}
