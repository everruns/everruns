// Gemini Decisions API (Gemini Decision 1).
//
// Google's decision endpoint lives beside the chat API on the same key and
// base URL: `POST {base}/decisions` with `x-goog-api-key`, where `base` is
// `https://generativelanguage.googleapis.com/v1beta`. Docs:
// https://aistudio.google.com/docs/decisions-api
//
// The primitives are System One's (noul, choice, score, with the same answer
// fields), but the envelope differs, so this module translates rather than
// reusing the System One body:
// - the content is `input` (a string or Gemini `Content` parts), not `state`;
//   structured state is sent as its JSON text;
// - noul criteria are `true_description` / `false_description`;
// - question keys must match `[A-Za-z0-9_-]{1,64}`, so questions go out under
//   positional names (`q0`, `q1`, ...) and caller ids never reach Google;
// - the response names no model and reports `total_input_tokens` /
//   `total_output_tokens` (always 0).
// Answers are then validated by the System One decoder, so a Gemini outcome
// passes the same calibration checks as TypeSafe's.
//
// Media (images, audio, video, PDFs) goes in `input` as Content parts after
// the text: `{"type": "image"|"audio"|"video"|"document", "data"|"uri",
// "mime_type"}`; the API names those five types only (probed 2026-11-12).
//
// Verified live on 2026-11-12: null choice descriptions are accepted, and
// score rubrics above ten levels are answered (the API allows 2-255). The
// request cap stays at System One's (20 questions, 10 levels) so every
// decision provider declares one shape. Safety blocks come back as HTTP 400.

use async_trait::async_trait;
use everruns_contracts::ProviderEndpoint;
use everruns_contracts::decision_driver::{
    DecisionDriver, DecisionDriverCapabilities, NativePrimitives,
};
use everruns_contracts::decisions::{
    DecisionMediaSource, DecisionOutcome, DecisionQuestion, DecisionRequest,
};
use everruns_contracts::error::{AgentLoopError, Result};
use serde_json::{Map, Value, json};

use crate::systemone::SystemOneDecisionDriver;

/// The model the Decisions API serves, sent when a request names none.
pub const DEFAULT_MODEL: &str = "gemini-decision-1";

/// Encoded media per request. The API takes 100 MB inline; this keeps one
/// call inside the transport's timeout and response budget.
pub const MAX_MEDIA_BYTES: usize = 20 * 1024 * 1024;

/// Gemini Decisions API driver.
#[derive(Clone, Debug, Default)]
pub struct GeminiDecisionDriver(SystemOneDecisionDriver);

impl GeminiDecisionDriver {
    /// Create the driver. Authentication comes from the provider endpoint.
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl DecisionDriver for GeminiDecisionDriver {
    fn id(&self) -> &str {
        "gemini-decisions"
    }

    fn capabilities(&self) -> DecisionDriverCapabilities {
        capabilities()
    }

    async fn evaluate(
        &self,
        endpoint: &ProviderEndpoint,
        request: DecisionRequest,
    ) -> Result<DecisionOutcome> {
        let url = endpoint
            .url("decisions")
            .ok_or_else(|| AgentLoopError::llm("Decision provider endpoint is required"))?;
        let request = with_model(request);
        let bytes = serde_json::to_vec(&encode(&request)?)
            .map_err(|_| AgentLoopError::llm("Invalid decision request"))?;
        let value = self.0.post_json(endpoint, url, bytes).await?;
        decode(&request, &value)
    }
}

/// System One's limits, plus media.
fn capabilities() -> DecisionDriverCapabilities {
    DecisionDriverCapabilities::new(NativePrimitives::ALL, true)
        .with_max_questions(20)
        .with_max_state_bytes(128 * 1024)
        .with_max_choice_options(255)
        .with_max_score_levels(10)
        .with_media(MAX_MEDIA_BYTES)
}

fn with_model(mut request: DecisionRequest) -> DecisionRequest {
    if request.model.as_deref().is_none_or(|m| m.trim().is_empty()) {
        request.model = Some(DEFAULT_MODEL.into());
    }
    request
}

/// The Decisions API body for `request`. Validates against the System One
/// limits first; a request without a model asks [`DEFAULT_MODEL`].
pub fn encode(request: &DecisionRequest) -> Result<Value> {
    let request = with_model(request.clone());
    // The System One rules cover the questions; media is Gemini's own.
    let mut questions_only = request.clone();
    questions_only.media.clear();
    crate::systemone::evaluation(&questions_only)?;
    capabilities().check("gemini", &request)?;
    let text = match &request.state {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    };
    let input = if request.media.is_empty() {
        Value::String(text)
    } else {
        let mut parts = vec![json!({"type": "text", "text": text})];
        for media in &request.media {
            let mut part =
                json!({"type": media_kind(&media.mime_type)?, "mime_type": media.mime_type});
            match &media.source {
                DecisionMediaSource::Data(data) => part["data"] = data.clone().into(),
                DecisionMediaSource::Uri(uri) => part["uri"] = uri.clone().into(),
            }
            parts.push(part);
        }
        Value::Array(parts)
    };
    let mut questions = Map::new();
    for (index, (_, question)) in request.questions.iter().enumerate() {
        let body = match question {
            DecisionQuestion::Noul {
                instructions,
                yes,
                no,
            } => {
                let mut criteria = Map::new();
                if let Some(yes) = yes {
                    criteria.insert("true_description".into(), yes.clone().into());
                }
                if let Some(no) = no {
                    criteria.insert("false_description".into(), no.clone().into());
                }
                let mut body = json!({"type": "noul", "instructions": instructions});
                if !criteria.is_empty() {
                    body["criteria"] = Value::Object(criteria);
                }
                body
            }
            DecisionQuestion::Choice {
                instructions,
                options,
            } => json!({
                "type": "choice",
                "instructions": instructions,
                "criteria": options
                    .iter()
                    .map(|(key, about)| (key.clone(), about.clone().map_or(Value::Null, Value::String)))
                    .collect::<Map<String, Value>>(),
            }),
            DecisionQuestion::Score {
                instructions,
                levels,
            } => json!({"type": "score", "instructions": instructions, "criteria": levels}),
        };
        questions.insert(format!("q{index}"), body);
    }
    Ok(json!({"model": request.model, "input": input, "questions": questions}))
}

/// The Content part type for a media type.
fn media_kind(mime_type: &str) -> Result<&'static str> {
    let mime = mime_type.to_ascii_lowercase();
    Ok(match mime.split_once('/').map(|(kind, _)| kind) {
        Some("image") => "image",
        Some("audio") => "audio",
        Some("video") => "video",
        _ if mime == "application/pdf" => "document",
        _ => {
            return Err(AgentLoopError::llm(format!(
                "Gemini decisions read images, audio, video and PDFs, not {mime_type}"
            )));
        }
    })
}

/// Validate a Decisions API reply to `request` into a calibrated outcome.
pub fn decode(request: &DecisionRequest, value: &Value) -> Result<DecisionOutcome> {
    let answers = value
        .get("answers")
        .and_then(Value::as_object)
        .ok_or_else(|| AgentLoopError::llm("Invalid decision response"))?;
    if answers.len() != request.questions.len() {
        return Err(AgentLoopError::llm("Incomplete decision response"));
    }
    let mut by_id = Map::new();
    for (index, (id, _)) in request.questions.iter().enumerate() {
        let answer = answers
            .get(&format!("q{index}"))
            .ok_or_else(|| AgentLoopError::llm("Missing decision answer"))?;
        by_id.insert(id.clone(), answer.clone());
    }
    let usage = &value["usage"];
    let system_one = json!({
        "model": request.model.as_deref().unwrap_or(DEFAULT_MODEL),
        "answers": by_id,
        "usage": {
            "input_tokens": usage["total_input_tokens"].as_u64().unwrap_or(0),
            "output_tokens": usage["total_output_tokens"].as_u64().unwrap_or(0),
        },
    });
    crate::systemone::decode(request, system_one)
}

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_contracts::decisions::DecisionAnswer;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// The live response from 2026-11-12, under positional keys.
    fn live_response() -> Value {
        json!({
            "answers": {
                "q0": {"confidence": 0.978, "noul": 0.978, "type": "noul"},
                "q1": {
                    "confidence": 0.941, "choice": "billing", "type": "choice",
                    "probabilities": {"technical": 0.059, "billing": 0.941}
                },
                "q2": {
                    "confidence": 0.787, "score": 1.7775666217177057, "type": "score",
                    "probabilities": {"2": 0.787, "1": 0.2035, "0": 0.0095},
                    "legend": {"1": "Important", "0": "Routine", "2": "Urgent"}
                }
            },
            "usage": {
                "total_tokens": 213, "total_input_tokens": 213,
                "input_tokens_by_modality": [{"modality": "text", "tokens": 213}],
                "total_output_tokens": 0
            }
        })
    }

    fn request() -> DecisionRequest {
        DecisionRequest::new("My payment failed twice today.")
            .ask(
                "is urgent?",
                DecisionQuestion::Noul {
                    instructions: "Is it urgent?".into(),
                    yes: Some("needs action today".into()),
                    no: None,
                },
            )
            .ask(
                "team",
                DecisionQuestion::Choice {
                    instructions: "Which team?".into(),
                    options: vec![
                        ("billing".into(), Some("Payments".into())),
                        ("technical".into(), None),
                    ],
                },
            )
            .ask(
                "priority",
                DecisionQuestion::score("Priority?", ["Routine", "Important", "Urgent"]),
            )
    }

    #[test]
    fn encodes_the_gemini_envelope_under_positional_keys() {
        let body = encode(&request()).unwrap();
        assert_eq!(body["model"], DEFAULT_MODEL);
        assert_eq!(body["input"], "My payment failed twice today.");
        assert!(body.get("state").is_none());
        let questions = body["questions"].as_object().unwrap();
        assert_eq!(questions.keys().collect::<Vec<_>>(), ["q0", "q1", "q2"]);
        assert_eq!(
            body["questions"]["q0"]["criteria"],
            json!({"true_description": "needs action today"})
        );
        assert_eq!(body["questions"]["q1"]["criteria"]["billing"], "Payments");
        assert!(body["questions"]["q1"]["criteria"]["technical"].is_null());
        assert_eq!(body["questions"]["q2"]["criteria"][2], "Urgent");

        let structured = DecisionRequest::new(json!({"order": 7}))
            .model("gemini-decision-1")
            .ask("q", DecisionQuestion::noul("Is it?"));
        let body = encode(&structured).unwrap();
        assert_eq!(body["input"], r#"{"order":7}"#);
        assert!(body["questions"]["q0"].get("criteria").is_none());
    }

    #[test]
    fn media_follows_the_text_as_typed_content_parts() {
        use everruns_contracts::decisions::DecisionMedia;
        let with_media = request()
            .attach(DecisionMedia::bytes("image/png", b"png"))
            .attach(DecisionMedia::uri(
                "application/pdf",
                "https://example.com/a.pdf",
            ));
        let body = encode(&with_media).unwrap();
        assert_eq!(
            body["input"],
            json!([
                {"type": "text", "text": "My payment failed twice today."},
                {"type": "image", "mime_type": "image/png", "data": "cG5n"},
                {"type": "document", "mime_type": "application/pdf", "uri": "https://example.com/a.pdf"},
            ])
        );
        let zip = request().attach(DecisionMedia::bytes("application/zip", b"zip"));
        assert!(
            encode(&zip)
                .unwrap_err()
                .to_string()
                .contains("not application/zip")
        );
        let huge = request().attach(DecisionMedia::bytes(
            "video/mp4",
            vec![0u8; MAX_MEDIA_BYTES],
        ));
        assert!(
            encode(&huge)
                .unwrap_err()
                .to_string()
                .contains("bytes of media")
        );
    }

    #[test]
    fn rejects_incomplete_or_uncalibrated_answers() {
        let request = request();
        let mut missing = live_response();
        missing["answers"].as_object_mut().unwrap().remove("q2");
        assert!(decode(&request, &missing).is_err());
        let mut skewed = live_response();
        skewed["answers"]["q1"]["probabilities"]["billing"] = json!(0.5);
        assert!(decode(&request, &skewed).is_err());
        assert!(decode(&request, &json!({"error": {"code": "safety"}})).is_err());
    }

    #[tokio::test]
    async fn answers_through_the_gemini_key_and_maps_ids_back() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1beta/decisions"))
            .and(header("x-goog-api-key", "gemini-key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(live_response()))
            .expect(1)
            .mount(&server)
            .await;
        let provider = crate::gemini::provider("gemini", "gemini-key")
            .base_url(format!("{}/v1beta", server.uri()));
        let outcome = GeminiDecisionDriver::new()
            .evaluate(provider.endpoint(), request())
            .await
            .unwrap();
        assert_eq!(outcome.model, DEFAULT_MODEL);
        assert!(outcome.calibrated);
        assert_eq!(outcome.usage.input_tokens, 213);
        assert!(
            outcome
                .get("is urgent?")
                .unwrap()
                .probability_yes()
                .unwrap()
                > 0.9
        );
        let DecisionAnswer::Choice { selected, .. } = outcome.get("team").unwrap() else {
            panic!("expected a choice");
        };
        assert_eq!(selected, "billing");
        let DecisionAnswer::Score { score, .. } = outcome.get("priority").unwrap() else {
            panic!("expected a score");
        };
        assert!((score - 1.7776).abs() < 1e-3);
    }

    #[tokio::test]
    async fn surfaces_safety_blocks_without_their_body() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(400).set_body_json(json!({
                "error": {"code": "safety", "message": "blocked"}
            })))
            .mount(&server)
            .await;
        let provider = crate::gemini::provider("gemini", "key").base_url(server.uri());
        let error = GeminiDecisionDriver::new()
            .evaluate(provider.endpoint(), request())
            .await
            .unwrap_err();
        assert!(error.to_string().contains("HTTP 400"), "{error}");
    }
}
