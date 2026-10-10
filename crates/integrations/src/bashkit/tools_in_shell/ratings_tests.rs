//! A tool with no risk hints is rated once, and a "changes things" rating makes
//! the approval gate ask before it.

use super::*;
use everruns_contracts::runtime::decisions::{
    DecisionAnswer, DecisionOutcome, DecisionRequest, DecisionUsage, DecisionsService,
};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Answers every rating with one probability and counts the requests.
struct Ratings {
    changes: f64,
    asked: AtomicUsize,
}

#[async_trait]
impl DecisionsService for Ratings {
    fn is_configured(&self) -> bool {
        true
    }

    async fn evaluate(
        &self,
        request: DecisionRequest,
    ) -> everruns_contracts::error::Result<DecisionOutcome> {
        self.asked.fetch_add(1, Ordering::SeqCst);
        assert_eq!(request.questions.len(), 1);
        Ok(DecisionOutcome {
            model: "test".into(),
            answers: BTreeMap::from([(
                "changes".to_string(),
                DecisionAnswer::Noul {
                    probability: self.changes,
                },
            )]),
            usage: DecisionUsage::default(),
            calibrated: true,
            attribution: None,
        })
    }
}

fn rated_context(changes: f64) -> (ToolContext, Arc<Policy>, Arc<Ratings>) {
    let policy = Arc::new(Policy {
        approval_for_destructive: true,
        ..Policy::default()
    });
    let ratings = Arc::new(Ratings {
        changes,
        asked: AtomicUsize::new(0),
    });
    let mut context = context(Some(policy.clone()), true);
    context.decisions = Some(ratings.clone());
    (context, policy, ratings)
}

#[tokio::test]
async fn a_tool_rated_as_changing_things_needs_approval() {
    let (context, policy, ratings) = rated_context(0.9);
    let output = run("tools web-fetch repo=a/b", &context).await;

    assert_eq!(
        output["tools"]["stopped"]["reason"], "needs_approval",
        "{output}"
    );
    assert_eq!(
        output["risk"], "rated_changes",
        "the card says the rating, not the tool, asked"
    );
    assert!(
        policy.after.lock().unwrap().is_empty(),
        "the call never ran"
    );
    assert_eq!(ratings.asked.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_tool_is_rated_once_and_a_no_changes_nothing() {
    let (context, policy, ratings) = rated_context(0.1);
    let output = run(
        "tools github list-pulls > /dev/null\ntools github list-pulls > /dev/null\ntools read-notes > /dev/null",
        &context,
    )
    .await;

    assert_eq!(output["exit_code"], 0, "{output}");
    assert_eq!(
        policy.after.lock().unwrap().as_slice(),
        [
            "mcp_github__list_pulls",
            "mcp_github__list_pulls",
            "read_notes"
        ]
    );
    assert_eq!(
        ratings.asked.load(Ordering::SeqCst),
        1,
        "rated once; a tool with hints is never rated"
    );
}

#[tokio::test]
async fn a_plan_shows_the_rating_before_anything_runs() {
    // Ratings are cached per process, so this test rates a tool no other test
    // here rates, which keeps the request counts above exact.
    let policy = Arc::new(Policy {
        approval_for_destructive: true,
        previews: true,
        ..Policy::default()
    });
    let ratings = Arc::new(Ratings {
        changes: 0.9,
        asked: AtomicUsize::new(0),
    });
    let mut context = context(Some(policy.clone()), true);
    context.decisions = Some(ratings.clone());
    let output = run("tools plan 'tools github get-issue number=7'", &context).await;

    let plan: Value = serde_json::from_str(output["stdout"].as_str().unwrap()).unwrap();
    assert_eq!(plan["calls"][0]["risk"], "needs_approval", "{plan}");
    assert_eq!(plan["calls"][0]["why"], "rated_changes", "{plan}");
    assert!(policy.after.lock().unwrap().is_empty(), "nothing ran");
}
