//! OpenRouter's alpha extraction route. The route, not output modality, selects
//! this service: some models also offer chat or decisions.
use super::{client, invalid, json};
use async_trait::async_trait;
use everruns_contracts::error::Result;
use everruns_contracts::extraction_driver::{ExtractionDriver, ExtractionDriverCapabilities};
use everruns_contracts::extractions::{ExtractionOutcome, ExtractionRequest};
use everruns_contracts::model_discovery::DiscoveredModel;
use everruns_contracts::{ProviderEndpoint, model::ModelProfile};

#[derive(Clone, Debug)]
pub struct OpenRouterExtractionDriver {
    http: Option<reqwest::Client>,
}
impl Default for OpenRouterExtractionDriver {
    fn default() -> Self {
        Self { http: client() }
    }
}

fn route(endpoint: &ProviderEndpoint) -> Result<String> {
    let base = endpoint
        .base_url()
        .ok_or_else(|| invalid("Extraction endpoint is required"))?;
    let mut url = reqwest::Url::parse(base).map_err(|_| invalid("Invalid extraction endpoint"))?;
    let path = url.path().trim_end_matches('/');
    let prefix = path.strip_suffix("/v1").unwrap_or(path);
    url.set_path(&format!("{prefix}/alpha/extractions"));
    url.set_query(None);
    url.set_fragment(None);
    Ok(url.to_string())
}

/// Convert account routing to OpenRouter's provider-options envelope, without
/// ever forwarding an internal provider key as an upstream routing directive.
fn encode(request: &ExtractionRequest) -> Result<serde_json::Value> {
    request.validate()?;
    if request.model.as_deref().is_none_or(|s| s.trim().is_empty()) {
        return Err(invalid("Extraction model is required"));
    }
    let mut value =
        serde_json::to_value(request).map_err(|_| invalid("Invalid extraction request"))?;
    let object = value
        .as_object_mut()
        .ok_or_else(|| invalid("Invalid extraction request"))?;
    object.remove("provider");
    object.remove("provider_options");
    if !request.provider_options.is_empty() {
        object.insert(
            "provider".into(),
            serde_json::json!({"options": request.provider_options}),
        );
    }
    Ok(value)
}

#[async_trait]
impl ExtractionDriver for OpenRouterExtractionDriver {
    fn id(&self) -> &str {
        "openrouter-extractions"
    }
    fn capabilities(&self) -> ExtractionDriverCapabilities {
        ExtractionDriverCapabilities {
            max_fields: Some(128),
            max_state_bytes: Some(2 * 1024 * 1024),
            max_asset_bytes: Some(12 * 1024 * 1024),
            ..ExtractionDriverCapabilities::all()
        }
    }
    async fn extract(
        &self,
        endpoint: &ProviderEndpoint,
        request: ExtractionRequest,
    ) -> Result<ExtractionOutcome> {
        self.capabilities().check(self.id(), &request)?;
        let body = encode(&request)?;
        let value = json(
            self.http
                .as_ref()
                .ok_or_else(|| invalid("Extraction HTTP client unavailable"))?,
            endpoint,
            reqwest::Method::POST,
            route(endpoint)?,
            Some(body),
        )
        .await?;
        let result: ExtractionOutcome =
            serde_json::from_value(value).map_err(|_| invalid("Invalid extraction response"))?;
        result.validate(&request)?;
        Ok(result)
    }
    async fn list_models(
        &self,
        endpoint: &ProviderEndpoint,
    ) -> Result<Option<Vec<DiscoveredModel>>> {
        let Some(url) = endpoint
            .url("models?endpoints=/api/alpha/extractions")
            .filter(|u| everruns_contracts::openai_protocol::url_host_eq(u, "openrouter.ai"))
        else {
            return Ok(None);
        };
        let value = json(
            self.http
                .as_ref()
                .ok_or_else(|| invalid("Extraction HTTP client unavailable"))?,
            endpoint,
            reqwest::Method::GET,
            url,
            None,
        )
        .await?;
        let data = value
            .get("data")
            .and_then(serde_json::Value::as_array)
            .ok_or_else(|| invalid("Invalid extraction model catalog"))?;
        let mut models = Vec::with_capacity(data.len());
        for model in data {
            let id = model
                .get("id")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| invalid("Extraction model has no id"))?;
            let mut profile = ModelProfile::minimal(id);
            profile.structured_output = true;
            models.push(DiscoveredModel {
                model_id: id.into(),
                display_name: model
                    .get("name")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned),
                created_at: None,
                owned_by: None,
                capabilities: vec!["extractions".into()],
                discovered_profile: Some(profile),
            });
        }
        Ok(Some(models))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_contracts::{BearerAuth, extractions::ExtractionField};
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{body_json, header, method, path},
    };
    fn request() -> ExtractionRequest {
        let mut r = ExtractionRequest::new("🙂 INV-42").field("invoice", ExtractionField::text());
        r.model = Some("cloudflare/clairic".into());
        r
    }
    #[tokio::test]
    async fn alpha_endpoint_preserves_unicode_evidence_and_unknown_confidence() {
        let server = MockServer::start().await;
        let endpoint = ProviderEndpoint::from_parts(
            format!("{}/api/v1", server.uri()),
            BearerAuth::new("test"),
        );
        Mock::given(method("POST")).and(path("/api/alpha/extractions")).and(header("authorization","Bearer test"))
            .and(body_json(serde_json::json!({"model":"cloudflare/clairic","state":"🙂 INV-42","fields":{"invoice":{"type":"text"}}})))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"model":"cloudflare/clairic","answers":{"invoice":{"type":"text","value":"INV-42","span":[2,8]}}}))).expect(1).mount(&server).await;
        let result = OpenRouterExtractionDriver::default()
            .extract(&endpoint, request())
            .await
            .unwrap();
        assert_eq!(result.answers["invoice"].confidence, None);
        assert_eq!(result.answers["invoice"].span, Some([2, 8]));
    }
    #[tokio::test]
    async fn rejects_wrong_answer_type_and_redacts_error_body() {
        let server = MockServer::start().await;
        let endpoint = ProviderEndpoint::from_parts(
            format!("{}/api/v1", server.uri()),
            BearerAuth::new("test"),
        );
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(400).set_body_string("secret document"))
            .mount(&server)
            .await;
        let error = OpenRouterExtractionDriver::default()
            .extract(&endpoint, request())
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("400"));
        assert!(!error.contains("secret"));
        let result: ExtractionOutcome = serde_json::from_value(
            serde_json::json!({"model":"m","answers":{"invoice":{"type":"text","value":42}}}),
        )
        .unwrap();
        assert!(result.validate(&request()).is_err());
    }
    #[test]
    fn account_id_is_not_upstream_provider_routing() {
        let mut r = request();
        r.provider = Some("my-account".into());
        r.provider_options.insert(
            "cloudflare".into(),
            serde_json::json!({"return_state_text":true}),
        );
        let body = encode(&r).unwrap();
        assert_eq!(
            body["provider"],
            serde_json::json!({"options":{"cloudflare":{"return_state_text":true}}})
        );
        assert!(body.get("provider_options").is_none());
    }
}
