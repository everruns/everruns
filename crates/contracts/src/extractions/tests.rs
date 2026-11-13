use super::*;
use serde_json::json;

fn outcome(value: Value) -> ExtractionOutcome {
    serde_json::from_value(value).unwrap()
}

#[test]
fn request_rejects_foreign_keys_and_invalid_options() {
    assert!(
        serde_json::from_value::<ExtractionField>(json!({"type":"text","questions":{}})).is_err()
    );
    assert!(
        serde_json::from_value::<ExtractionRequest>(
            json!({"state":"x","fields":{},"questions":{}})
        )
        .is_err()
    );
    let mut f = ExtractionField::text();
    f.decimals = Some(2);
    assert!(f.validate().is_err());
    let mut f = ExtractionField::number();
    f.null_threshold = Some(f64::NAN);
    assert!(f.validate().is_err());
    f.null_threshold = Some(0.5);
    f.decimals = Some(7);
    assert!(f.validate().is_err());
    assert!(ExtractionRequest::new("x").validate().is_err());
    assert!(
        ExtractionRequest::new("")
            .field("x", ExtractionField::text())
            .validate()
            .is_err()
    );
}

#[test]
fn calendar_and_forecast_bounds_are_checked() {
    let mut f = ExtractionField::new(ExtractionType::Forecast);
    f.min = Some(ExtractionBound::Number(0.0));
    f.max = Some(ExtractionBound::Number(10.0));
    f.horizon = Some(3);
    f.interval = Some(ForecastInterval::Day);
    assert!(f.validate().is_ok());
    f.horizon = Some(65);
    assert!(f.validate().is_err());
    f.horizon = Some(3);
    f.step = Some(ExtractionStep::Number(0.0));
    assert!(f.validate().is_err());
    f.step = None;
    f.min = Some(ExtractionBound::Date("2026-02-30".into()));
    f.max = Some(ExtractionBound::Date("2026-03-01".into()));
    assert!(f.validate().is_err());
}

#[test]
fn evidence_uses_codepoints_not_bytes_and_preserves_unknown_confidence() {
    let req = ExtractionRequest::new("🦀 café").field("word", ExtractionField::text());
    let out = outcome(
        json!({"model":"m","answers":{"word":{"type":"text","value":"café","span":[2,6]}}}),
    );
    assert!(out.validate(&req).is_ok());
    assert_eq!(out.answers["word"].confidence, None);
    assert!(
        serde_json::to_value(&out).unwrap()["answers"]["word"]
            .get("confidence")
            .is_none()
    );
    let mut bad = out;
    bad.answers.get_mut("word").unwrap().span = Some([2, 9]);
    assert!(bad.validate(&req).is_err());
}

#[test]
fn null_answers_are_distinct_from_low_confidence_and_bad_types() {
    let req = ExtractionRequest::new("no total").field("total", ExtractionField::number());
    let mut out = outcome(
        json!({"model":"m","answers":{"total":{"type":"number","value":null,"guess":12.0,"p_null":0.9}}}),
    );
    assert!(out.validate(&req).is_ok());
    out.answers.get_mut("total").unwrap().value = json!("12");
    assert!(out.validate(&req).is_err());
    out.answers.get_mut("total").unwrap().value = json!(12);
    out.answers.get_mut("total").unwrap().confidence = Some(2.0);
    assert!(out.validate(&req).is_err());
}

#[test]
fn list_offsets_allow_missing_items_but_must_align() {
    let req = ExtractionRequest::new("A B")
        .field("names", ExtractionField::list(ExtractionItemType::Text));
    let mut out = outcome(
        json!({"model":"m","answers":{"names":{"type":"list","value":["A","B"],"spans":[[0,1],null]}}}),
    );
    assert!(out.validate(&req).is_ok());
    out.answers.get_mut("names").unwrap().spans = Some(vec![Some([0, 1])]);
    assert!(out.validate(&req).is_err());
}

#[test]
fn asset_validation_is_inline_and_referenced() {
    let mut req = ExtractionRequest::new("photo").field("name", ExtractionField::text());
    req.assets.insert(
        "photo".into(),
        ExtractionAsset {
            kind: Some(ExtractionAssetType::Image),
            format: ExtractionAssetFormat::Png,
            data_base64: "aGk=".into(),
        },
    );
    assert!(req.validate().is_ok());
    req.state = "unrelated".into();
    assert!(req.validate().is_err());
    req.state.clear();
    assert!(req.validate().is_ok());
    req.assets.get_mut("photo").unwrap().data_base64 = "https://internal/secret".into();
    assert!(req.validate().is_err());
}

struct TestExtractionDriver;
#[async_trait]
impl crate::extraction_driver::ExtractionDriver for TestExtractionDriver {
    fn id(&self) -> &str {
        "test"
    }
    fn capabilities(&self) -> crate::extraction_driver::ExtractionDriverCapabilities {
        crate::extraction_driver::ExtractionDriverCapabilities::all()
    }
    async fn extract(
        &self,
        _: &crate::runtime_provider::ProviderEndpoint,
        request: ExtractionRequest,
    ) -> Result<ExtractionOutcome> {
        Ok(outcome(
            json!({"model":request.model.unwrap_or_default(),"answers":{"name":{"type":"text","value":"Ada"}}}),
        ))
    }
    async fn list_models(
        &self,
        _: &crate::runtime_provider::ProviderEndpoint,
    ) -> Result<Option<Vec<crate::model_discovery::DiscoveredModel>>> {
        Ok(Some(vec![crate::model_discovery::DiscoveredModel {
            model_id: "extract-v1".into(),
            display_name: None,
            created_at: None,
            owned_by: None,
            capabilities: vec!["extractions".into()],
            discovered_profile: None,
        }]))
    }
}
#[tokio::test]
async fn provider_routes_extractions_without_a_chat_or_decisions_service() {
    let provider =
        crate::RuntimeProvider::services("account").with_extractions(TestExtractionDriver);
    assert!(provider.supports_service(crate::ServiceKind::Extractions));
    assert!(!provider.supports_service(crate::ServiceKind::Decisions));
    let req = ExtractionRequest::new("Ada").field("name", ExtractionField::text());
    assert_eq!(
        provider.extract(req.clone()).await.unwrap().answers["name"].value,
        json!("Ada")
    );
    let models = provider.list_models().await.unwrap().unwrap();
    assert_eq!(models[0].capabilities, ["extractions"]);
    let mut wrong_account = req;
    wrong_account.provider = Some("other".into());
    assert!(provider.extract(wrong_account).await.is_err());
}

#[test]
fn capabilities_reject_unsupported_fields_and_assets_before_dispatch() {
    let caps = crate::extraction_driver::ExtractionDriverCapabilities {
        field_types: vec![ExtractionType::Text],
        ..Default::default()
    };
    assert!(
        caps.check(
            "test",
            &ExtractionRequest::new("x").field("n", ExtractionField::number())
        )
        .is_err()
    );
    assert!(
        caps.check(
            "test",
            &ExtractionRequest::new("x").field("n", ExtractionField::text())
        )
        .is_ok()
    );
}

struct SharedChatCatalog;
#[async_trait]
impl crate::ChatDriver for SharedChatCatalog {
    async fn chat_completion_stream(
        &self,
        _: &crate::ProviderEndpoint,
        _: Vec<crate::Message>,
        _: &crate::LlmCallConfig,
    ) -> Result<crate::LlmResponseStream> {
        unreachable!("catalog-only test")
    }
    async fn list_models(
        &self,
        _: &crate::ProviderEndpoint,
    ) -> Result<Option<Vec<crate::model_discovery::DiscoveredModel>>> {
        let mut profile = crate::model::ModelProfile::minimal("extract-v1");
        profile.structured_output = true;
        Ok(Some(vec![crate::model_discovery::DiscoveredModel {
            model_id: "extract-v1".into(),
            display_name: Some("Original chat identity".into()),
            created_at: None,
            owned_by: Some("original-owner".into()),
            capabilities: vec!["chat".into(), "structured_output".into()],
            discovered_profile: Some(profile),
        }]))
    }
}

#[tokio::test]
async fn extraction_catalog_coalesces_shared_model_without_erasing_chat_metadata() {
    let provider = crate::RuntimeProvider::new("account", SharedChatCatalog)
        .with_extractions(TestExtractionDriver);
    let models = provider.list_models().await.unwrap().unwrap();
    assert_eq!(models.len(), 1, "one database identity per provider/model");
    let model = &models[0];
    assert_eq!(model.model_id, "extract-v1");
    assert_eq!(
        model.display_name.as_deref(),
        Some("Original chat identity")
    );
    assert_eq!(model.owned_by.as_deref(), Some("original-owner"));
    assert_eq!(
        model.capabilities,
        ["chat", "structured_output", "extractions"]
    );
    assert!(model.discovered_profile.as_ref().unwrap().structured_output);
}

#[test]
fn null_list_evidence_aligns_with_retained_guess_only() {
    let request = ExtractionRequest::new("A B")
        .field("names", ExtractionField::list(ExtractionItemType::Text));
    let mut result = outcome(json!({"model":"m","answers":{"names":{
        "type":"list","value":null,"guess":["A","B"],"p_null":0.9,
        "spans":[[0,1],null],"confidences":[0.5,null]
    }}}));
    assert!(result.validate(&request).is_ok());
    result.answers.get_mut("names").unwrap().guess = Some(json!(["A"]));
    assert!(result.validate(&request).is_err());
    result.answers.get_mut("names").unwrap().guess = None;
    assert!(result.validate(&request).is_err());
}

#[test]
fn encoded_size_limits_precede_base64_content_validation() {
    let caps = crate::extraction_driver::ExtractionDriverCapabilities {
        max_asset_bytes: Some(4),
        ..crate::extraction_driver::ExtractionDriverCapabilities::all()
    };
    let request = ExtractionRequest::new("")
        .field("x", ExtractionField::text())
        .asset(
            "photo",
            ExtractionAsset {
                kind: None,
                format: ExtractionAssetFormat::Png,
                data_base64: "this-is-not-base64-and-too-large".into(),
            },
        );
    let error = caps.check("test", &request).unwrap_err().to_string();
    assert!(error.contains("size limit"), "{error}");
}

#[test]
fn request_asset_count_is_bounded_before_content_decoding() {
    let mut request = ExtractionRequest::new("").field("name", ExtractionField::text());
    for i in 0..33 {
        request.assets.insert(
            i.to_string(),
            ExtractionAsset {
                kind: None,
                format: ExtractionAssetFormat::Png,
                data_base64: "invalid-base64".into(),
            },
        );
    }
    let error = request.validate().unwrap_err().to_string();
    assert!(error.contains("runtime limits"), "{error}");
}
