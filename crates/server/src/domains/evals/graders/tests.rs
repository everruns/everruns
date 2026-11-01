use super::*;
use crate::domains::observers::judge::JudgeResult;
use everruns_contracts::typed_id::ModelId;
use everruns_core::DecisionAnswer;
use everruns_core::message::{AnnotationSource, VerificationVerdict};
use std::collections::BTreeMap;
use std::sync::Mutex;

fn graders() -> Graders {
    Graders::new(Arc::new(StorageBackend::test_database()))
}

fn input<'a>(answer: &'a str, tools: &'a [String]) -> GradeInput<'a> {
    GradeInput {
        input_message: "Can I get a refund?",
        final_answer: answer,
        tool_calls: tools,
        turns: 1,
        ..Default::default()
    }
}

fn rule(scorer: &Scorer, answer: &str) -> Score {
    score_rule(scorer, &input(answer, &[])).expect("pure rule")
}

#[test]
fn contains_rule() {
    let scorer = Scorer::Contains {
        text: "hello".into(),
        weight: 1.0,
    };
    assert!(rule(&scorer, "say hello world").pass);
    assert!(!rule(&scorer, "goodbye").pass);
}

#[test]
fn tool_called_rule() {
    let scorer = Scorer::ToolCalled {
        tool: "web_fetch".into(),
        min: 2,
        weight: 1.0,
    };
    let calls = vec!["web_fetch".to_string(), "web_fetch".to_string()];
    assert!(score_rule(&scorer, &input("", &calls)).unwrap().pass);
    assert!(!score_rule(&scorer, &input("", &calls[..1])).unwrap().pass);
}

#[test]
fn turns_and_tokens_within() {
    let turns = Scorer::TurnsWithin {
        max: 3,
        weight: 1.0,
    };
    let tokens = Scorer::TokensWithin {
        max: 1000,
        weight: 1.0,
    };
    let mut slice = input("", &[]);
    slice.turns = 3;
    slice.tokens = 1000;
    assert!(score_rule(&turns, &slice).unwrap().pass);
    assert!(score_rule(&tokens, &slice).unwrap().pass);
    slice.turns = 4;
    slice.tokens = 1001;
    assert!(!score_rule(&turns, &slice).unwrap().pass);
    let over = score_rule(&tokens, &slice).unwrap();
    assert!(!over.pass);
    assert_eq!(over.reason, "Tokens: 1001 (max: 1000)");
}

#[test]
fn json_schema_validates_the_shape_not_just_the_syntax() {
    let scorer = Scorer::JsonSchema {
        schema: serde_json::json!({
            "type": "object",
            "required": ["refund"],
            "properties": {"refund": {"type": "boolean"}}
        }),
        weight: 1.0,
    };
    assert!(rule(&scorer, r#"{"refund": true}"#).pass);
    assert!(rule(&scorer, "```json\n{\"refund\": false}\n```").pass);
    let wrong = rule(&scorer, r#"{"refund": "yes"}"#);
    assert!(!wrong.pass);
    assert!(wrong.reason.contains("does not match"), "{}", wrong.reason);
    assert!(!rule(&scorer, "not json").pass);
}

#[test]
fn invalid_regex_fails_with_a_reason() {
    let scorer = Scorer::Regex {
        pattern: "(".into(),
        weight: 1.0,
    };
    let score = rule(&scorer, "anything");
    assert!(!score.pass);
    assert!(score.reason.starts_with("Invalid pattern"));
}

#[test]
fn validate_rejects_scorers_that_could_never_grade() {
    let empty_rubric = Scorer::Judge {
        rubric: " ".into(),
        model_id: None,
        pass_threshold: 0.5,
        weight: 1.0,
    };
    assert!(empty_rubric.validate().is_err());
    let bad_threshold = Scorer::Decision {
        question: "Is it polite?".into(),
        yes: None,
        no: None,
        pass_threshold: 1.5,
        weight: 1.0,
    };
    assert!(bad_threshold.validate().is_err());
    let bad_regex = Scorer::Regex {
        pattern: "(".into(),
        weight: 1.0,
    };
    assert!(bad_regex.validate().is_err());
    let negative = Scorer::Contains {
        text: "x".into(),
        weight: -1.0,
    };
    assert!(negative.validate().is_err());
    let fine = Scorer::Decision {
        question: "Is it polite?".into(),
        yes: None,
        no: None,
        pass_threshold: 0.7,
        weight: 2.0,
    };
    assert!(fine.validate().is_ok());
}

#[test]
fn new_scorers_round_trip_through_json() {
    let decision: Scorer = serde_json::from_value(serde_json::json!({
        "type": "decision",
        "question": "Does the answer stay within policy?"
    }))
    .unwrap();
    assert_eq!(decision.kind(), "decision");
    assert!(matches!(
        decision,
        Scorer::Decision { pass_threshold, weight, .. } if pass_threshold == 0.5 && weight == 1.0
    ));
    let judge: Scorer = serde_json::from_value(serde_json::json!({
        "type": "judge",
        "rubric": "1 when correct"
    }))
    .unwrap();
    assert_eq!(judge.kind(), "judge");
}

// ---- model-backed graders ----

struct FakeJudge {
    value: f64,
}

#[async_trait]
impl JudgeClient for FakeJudge {
    async fn judge(
        &self,
        _org_id: i64,
        _model_id: Option<ModelId>,
        rubric: &str,
        evidence: &TurnEvidence,
    ) -> anyhow::Result<Option<JudgeResult>> {
        assert!(!rubric.is_empty());
        Ok(Some(JudgeResult {
            value: self.value,
            label: Some("ok".into()),
            reasoning: format!("judged '{}'", evidence.final_answer),
            input_tokens: Some(120),
            output_tokens: Some(30),
            cost_usd: Some(0.002),
        }))
    }
}

#[derive(Default)]
struct FakeDecisions {
    probability: f64,
    seen: Mutex<Vec<DecisionRequest>>,
}

#[async_trait]
impl DecisionsService for FakeDecisions {
    fn is_configured(&self) -> bool {
        true
    }

    async fn evaluate(
        &self,
        request: DecisionRequest,
    ) -> everruns_contracts::error::Result<DecisionOutcome> {
        self.seen.lock().unwrap().push(request);
        Ok(DecisionOutcome {
            model: "system-one".into(),
            answers: BTreeMap::from([(
                "grade".to_string(),
                DecisionAnswer::Noul {
                    probability: self.probability,
                },
            )]),
            usage: everruns_core::DecisionUsage {
                input_tokens: 40,
                output_tokens: 1,
            },
            calibrated: true,
            attribution: None,
        })
    }
}

struct Fixed(Arc<FakeDecisions>);

#[async_trait]
impl DecisionResolver for Fixed {
    async fn for_org(&self, _org_id: i64) -> Result<Arc<dyn DecisionsService>, GradeError> {
        Ok(self.0.clone())
    }
}

fn decision(threshold: f64) -> Scorer {
    Scorer::Decision {
        question: "Does the answer stay within the refund policy?".into(),
        yes: Some("It only promises what the policy allows".into()),
        no: None,
        pass_threshold: threshold,
        weight: 1.0,
    }
}

#[tokio::test]
async fn decision_scorer_uses_the_probability_of_yes() {
    let service = Arc::new(FakeDecisions {
        probability: 0.82,
        ..Default::default()
    });
    let graders = graders().with_decisions(Arc::new(Fixed(service.clone())));
    let tools = vec!["lookup_policy".to_string()];
    let slice = input("Yes, within 30 days.", &tools);

    let graded = graders.grade(1, &decision(0.7), &slice).await.unwrap();
    assert!(graded.score.pass);
    assert!((graded.score.value - 0.82).abs() < 1e-9);
    assert!(
        graded.score.reason.contains("calibrated"),
        "{}",
        graded.score.reason
    );
    assert_eq!(graded.usage.unwrap().input_tokens, 40);

    let failed = graders.grade(1, &decision(0.9), &slice).await.unwrap();
    assert!(!failed.score.pass);

    // The model sees the exchange, never the scorer's own bookkeeping.
    let seen = service.seen.lock().unwrap();
    let request = &seen[0];
    assert_eq!(request.state["agent_answer"], "Yes, within 30 days.");
    assert_eq!(request.state["user_message"], "Can I get a refund?");
    assert_eq!(request.state["tools_used"][0], "lookup_policy");
    assert_eq!(request.questions.len(), 1);
}

#[tokio::test]
async fn judge_scorer_reports_value_label_and_cost() {
    let graders = graders().with_judge(Arc::new(FakeJudge { value: 0.4 }));
    let scorer = Scorer::Judge {
        rubric: "1 when the refund terms are right".into(),
        model_id: None,
        pass_threshold: 0.5,
        weight: 1.0,
    };
    let graded = graders
        .grade(1, &scorer, &input("30 days", &[]))
        .await
        .unwrap();
    assert!(!graded.score.pass);
    assert_eq!(graded.score.reason, "judged '30 days'");
    assert_eq!(graded.label.as_deref(), Some("ok"));
    let usage = graded.usage.unwrap();
    assert_eq!((usage.input_tokens, usage.output_tokens), (120, 30));
    assert_eq!(usage.cost_usd, Some(0.002));
}

#[tokio::test]
async fn model_scorers_without_models_say_so() {
    let graders = graders();
    let judge = Scorer::Judge {
        rubric: "r".into(),
        model_id: None,
        pass_threshold: 0.5,
        weight: 1.0,
    };
    for scorer in [judge, decision(0.5)] {
        let error = graders
            .grade(1, &scorer, &input("a", &[]))
            .await
            .unwrap_err();
        assert!(matches!(error, GradeError::Unavailable(_)), "{error}");
        let score = error.into_score();
        assert!(!score.pass);
        assert!(score.reason.contains("unavailable"), "{}", score.reason);
    }
}

#[tokio::test]
async fn file_contains_needs_a_session() {
    let scorer = Scorer::FileContains {
        path: "/x".into(),
        text: "y".into(),
        weight: 1.0,
    };
    assert!(score_rule(&scorer, &input("", &[])).is_none());
    let error = graders()
        .grade(1, &scorer, &input("", &[]))
        .await
        .unwrap_err();
    assert!(matches!(error, GradeError::Unavailable(_)));
}

#[test]
fn usage_adds_up_and_keeps_known_cost() {
    let mut total = GraderUsage::default();
    total.add(&GraderUsage {
        input_tokens: 10,
        output_tokens: 2,
        cost_usd: None,
    });
    total.add(&GraderUsage {
        input_tokens: 5,
        output_tokens: 1,
        cost_usd: Some(0.01),
    });
    assert_eq!(total.input_tokens, 15);
    assert_eq!(total.output_tokens, 3);
    assert_eq!(total.cost_usd, Some(0.01));
}

// ---- citations ----

fn verified_annotation(status: VerificationStatus) -> TextAnnotation {
    TextAnnotation {
        start: 0,
        end: 5,
        origin: "citation_retrieval".to_string(),
        source: AnnotationSource {
            uri: "u".to_string(),
            title: None,
            snippet: None,
            location: None,
        },
        external_id: None,
        verified: Some(VerificationVerdict {
            status,
            score: Some(0.9),
        }),
    }
}

#[test]
fn citation_faithful_passes_when_all_entailed() {
    let anns = vec![
        verified_annotation(VerificationStatus::Entailed),
        verified_annotation(VerificationStatus::Entailed),
    ];
    let score = score_citation_faithful(&anns, 1, 0.8);
    assert!(score.pass);
    assert!((score.value - 1.0).abs() < 1e-9);
}

#[test]
fn citation_faithful_fails_below_threshold() {
    let anns = vec![
        verified_annotation(VerificationStatus::Entailed),
        verified_annotation(VerificationStatus::Unsupported),
    ];
    let score = score_citation_faithful(&anns, 1, 0.8);
    assert!(!score.pass);
    assert!((score.value - 0.5).abs() < 1e-9);
}

#[test]
fn citation_faithful_fails_on_insufficient_coverage() {
    let anns = vec![verified_annotation(VerificationStatus::Entailed)];
    let score = score_citation_faithful(&anns, 3, 0.5);
    assert!(!score.pass);
    assert_eq!(score.value, 0.0);
}

#[test]
fn citation_faithful_notes_missing_verdicts() {
    let mut ann = verified_annotation(VerificationStatus::Entailed);
    ann.verified = None;
    let score = score_citation_faithful(&[ann], 1, 0.8);
    assert!(!score.pass);
    assert!(score.reason.contains("enable citation_verification"));
}

#[test]
fn citation_pairs_render_claim_and_source() {
    let text = "The sky is blue. Grass is green.";
    let anns = vec![TextAnnotation {
        start: 0,
        end: 16,
        origin: "citation_retrieval".to_string(),
        source: AnnotationSource {
            uri: "doc://a".to_string(),
            title: None,
            snippet: Some("The sky appears blue due to Rayleigh scattering.".to_string()),
            location: None,
        },
        external_id: None,
        verified: None,
    }];
    let pairs = build_citation_pairs(text, &anns);
    assert!(pairs.contains("[1] CLAIM: The sky is blue."));
    assert!(pairs.contains("SOURCE (doc://a): The sky appears blue"));
}
