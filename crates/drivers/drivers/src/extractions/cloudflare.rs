//! Direct Workers AI Clairic extraction. This is not a chat-completions call.
use super::{client, invalid, json};
use async_trait::async_trait;
use everruns_contracts::extraction_driver::{ExtractionDriver, ExtractionDriverCapabilities};
use everruns_contracts::extractions::{
    ExtractionAssetFormat, ExtractionOutcome, ExtractionRequest, ExtractionType,
};
use everruns_contracts::{ProviderEndpoint, error::Result};

#[derive(Clone, Debug)]
pub struct CloudflareExtractionDriver {
    http: Option<reqwest::Client>,
}
impl Default for CloudflareExtractionDriver {
    fn default() -> Self {
        Self { http: client() }
    }
}

fn route(endpoint: &ProviderEndpoint) -> Result<String> {
    let mut url = reqwest::Url::parse(
        endpoint
            .base_url()
            .ok_or_else(|| invalid("Cloudflare extraction endpoint is required"))?,
    )
    .map_err(|_| invalid("Invalid Cloudflare extraction endpoint"))?;
    let path = url.path().trim_end_matches('/');
    let prefix = path.strip_suffix("/v1").unwrap_or(path);
    url.set_path(&format!("{prefix}/run/@cf/cloudflare/clairic"));
    url.set_query(None);
    url.set_fragment(None);
    Ok(url.to_string())
}
fn capabilities() -> ExtractionDriverCapabilities {
    ExtractionDriverCapabilities {
        documents: false,
        max_assets: Some(4),
        ..ExtractionDriverCapabilities::all()
    }
}
/// Named assets are an envelope convention, not Clairic reference syntax.
fn native_state(request: &ExtractionRequest) -> &str {
    if request.assets.len() == 1 && request.assets.contains_key(&request.state) {
        ""
    } else {
        &request.state
    }
}

fn encode(request: &ExtractionRequest) -> Result<serde_json::Value> {
    capabilities().check("cloudflare-extractions", request)?;
    if request
        .model
        .as_deref()
        .is_some_and(|m| !matches!(m, "clairic" | "@cf/cloudflare/clairic"))
    {
        return Err(invalid("Direct Cloudflare extraction requires Clairic"));
    }
    for (name, field) in &request.fields {
        let mut bytes = name.bytes();
        if name.len() > 64
            || !bytes
                .next()
                .is_some_and(|b| b.is_ascii_alphabetic() || b == b'_')
            || !bytes.all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-'))
        {
            return Err(invalid("Invalid Clairic field name"));
        }
        if field.null_threshold.is_some_and(|t| t <= 0.0)
            || (field.null_threshold.is_some()
                && matches!(
                    field.kind,
                    ExtractionType::List | ExtractionType::Estimate | ExtractionType::Forecast
                ))
        {
            return Err(invalid(
                "Clairic null_threshold requires a scalar field and a value above zero",
            ));
        }
    }
    for (name, value) in &request.provider_options {
        if !matches!(name.as_str(), "include_quantiles" | "return_state_text")
            || !value.is_boolean()
        {
            return Err(invalid("Unsupported Clairic extraction option"));
        }
    }
    // Clairic cannot select one positional image from a bare asset key. Keep
    // the documented envelope semantics instead of silently reading all images.
    if request.assets.len() >= 2
        && (request.assets.contains_key(&request.state)
            || request.fields.values().any(|field| {
                field
                    .instructions
                    .as_ref()
                    .is_some_and(|s| request.assets.contains_key(s))
            }))
    {
        return Err(invalid(
            "Clairic cannot resolve a whole-key image reference with multiple assets",
        ));
    }
    let mut body = serde_json::json!({"model":"clairic","state":native_state(request),"fields":request.fields});
    if let Some(date) = &request.reference_date {
        body["reference_date"] = date.clone().into();
    }
    if !request.provider_options.is_empty() {
        body["options"] = serde_json::to_value(&request.provider_options)
            .map_err(|_| invalid("Invalid Clairic options"))?;
    }
    let mut total = 0usize;
    let mut images = Vec::new();
    // Native Clairic has an ordered image list, not named assets. BTreeMap order
    // makes mapping deterministic; callers should describe image order in state.
    for asset in request.assets.values() {
        let decoded = asset.data_base64.len() / 4 * 3
            - asset
                .data_base64
                .bytes()
                .rev()
                .take_while(|b| *b == b'=')
                .count();
        total = total.saturating_add(decoded);
        if decoded > 4 * 1024 * 1024 || total > 8 * 1024 * 1024 {
            return Err(invalid("Clairic image size limit exceeded"));
        }
        let mime = match asset.format {
            ExtractionAssetFormat::Png => "image/png",
            ExtractionAssetFormat::Jpg | ExtractionAssetFormat::Jpeg => "image/jpeg",
            ExtractionAssetFormat::Webp => "image/webp",
            ExtractionAssetFormat::Pdf => {
                return Err(invalid("Clairic does not support PDF assets"));
            }
        };
        images.push(serde_json::json!({"content_type":mime,"base64":asset.data_base64}));
    }
    if !images.is_empty() {
        body["images"] = images.into();
    }
    if serde_json::to_vec(&body)
        .map_err(|_| invalid("Invalid Clairic request"))?
        .len()
        > 16 * 1024 * 1024
    {
        return Err(invalid("Clairic request body size limit exceeded"));
    }
    Ok(body)
}
fn decode(value: serde_json::Value, request: &ExtractionRequest) -> Result<ExtractionOutcome> {
    let value = if let Some(success) = value.get("success") {
        if success != &serde_json::Value::Bool(true) {
            return Err(invalid("Cloudflare extraction failed"));
        }
        value
            .get("result")
            .cloned()
            .ok_or_else(|| invalid("Missing Cloudflare extraction result"))?
    } else {
        value
    };
    let mut result: ExtractionOutcome = serde_json::from_value(value)
        .map_err(|_| invalid("Invalid Clairic extraction response"))?;
    result.provider = Some("cloudflare".into());
    // Spans index the text actually sent, not a sole asset-key placeholder.
    let mut native_request = request.clone();
    native_request.state = native_state(request).to_owned();
    result.validate(&native_request)?;
    Ok(result)
}
#[async_trait]
impl ExtractionDriver for CloudflareExtractionDriver {
    fn id(&self) -> &str {
        "cloudflare-extractions"
    }
    fn capabilities(&self) -> ExtractionDriverCapabilities {
        capabilities()
    }
    async fn list_models(
        &self,
        _endpoint: &ProviderEndpoint,
    ) -> Result<Option<Vec<everruns_contracts::model_discovery::DiscoveredModel>>> {
        let mut profile =
            everruns_contracts::model::ModelProfile::minimal("@cf/cloudflare/clairic");
        profile.structured_output = true;
        Ok(Some(vec![
            everruns_contracts::model_discovery::DiscoveredModel {
                model_id: "@cf/cloudflare/clairic".into(),
                display_name: Some("Clairic (preview)".into()),
                created_at: None,
                owned_by: Some("cloudflare".into()),
                capabilities: vec!["extractions".into()],
                discovered_profile: Some(profile),
            },
        ]))
    }
    async fn extract(
        &self,
        endpoint: &ProviderEndpoint,
        request: ExtractionRequest,
    ) -> Result<ExtractionOutcome> {
        let body = encode(&request)?;
        let result = json(
            self.http
                .as_ref()
                .ok_or_else(|| invalid("Extraction HTTP client unavailable"))?,
            endpoint,
            reqwest::Method::POST,
            route(endpoint)?,
            Some(body),
        )
        .await?;
        decode(result, &request)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use everruns_contracts::{
        BearerAuth,
        extractions::{ExtractionAsset, ExtractionField},
    };
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{body_json, header, method, path},
    };
    fn request() -> ExtractionRequest {
        ExtractionRequest::new("🙂 INV-42").field("invoice", ExtractionField::text())
    }
    #[tokio::test]
    async fn uses_native_route_auth_and_preserves_optional_evidence() {
        let server = MockServer::start().await;
        let endpoint = ProviderEndpoint::from_parts(
            format!("{}/accounts/a/ai/v1", server.uri()),
            BearerAuth::new("test"),
        );
        Mock::given(method("POST")).and(path("/accounts/a/ai/run/@cf/cloudflare/clairic")).and(header("authorization","Bearer test")).and(body_json(serde_json::json!({"model":"clairic","state":"🙂 INV-42","fields":{"invoice":{"type":"text"}}}))).respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"success":true,"result":{"model":"clairic","answers":{"invoice":{"type":"text","value":"INV-42","span":[2,8],"p_null":0.1}}}}))).expect(1).mount(&server).await;
        let result = CloudflareExtractionDriver::default()
            .extract(&endpoint, request())
            .await
            .unwrap();
        assert_eq!(result.answers["invoice"].confidence, None);
        assert_eq!(result.answers["invoice"].span, Some([2, 8]));
        assert_eq!(result.answers["invoice"].p_null, Some(0.1));
    }
    #[test]
    fn maps_images_options_and_rejects_unsupported_input() {
        let mut r = request();
        r.state = "invoice image".into();
        r.assets.insert(
            "image".into(),
            ExtractionAsset {
                kind: None,
                format: ExtractionAssetFormat::Png,
                data_base64: "aGk=".into(),
            },
        );
        r.provider_options
            .insert("return_state_text".into(), true.into());
        let body = encode(&r).unwrap();
        assert_eq!(body["images"][0]["content_type"], "image/png");
        assert_eq!(body["options"]["return_state_text"], true);
        r.assets.get_mut("image").unwrap().format = ExtractionAssetFormat::Pdf;
        assert!(encode(&r).is_err());
        r.assets.clear();
        r.model = Some("unknown".into());
        assert!(encode(&r).is_err());
        r.model = None;
        r.fields.get_mut("invoice").unwrap().null_threshold = Some(0.0);
        assert!(encode(&r).is_err());
    }
    fn image() -> ExtractionAsset {
        ExtractionAsset {
            kind: None,
            format: ExtractionAssetFormat::Png,
            data_base64: "aGk=".into(),
        }
    }
    #[test]
    fn sole_asset_key_and_empty_state_are_file_only() {
        let mut r = request();
        r.assets.insert("receipt".into(), image());
        r.state = "receipt".into();
        assert_eq!(encode(&r).unwrap()["state"], "");
        // A provider span must not be validated against the placeholder key.
        assert!(decode(serde_json::json!({"model":"clairic","answers":{"invoice":{"type":"text","value":"r","span":[0,1]}}}), &r).is_err());
        r.state.clear();
        assert_eq!(encode(&r).unwrap()["state"], "");
    }
    #[test]
    fn rejects_multi_image_whole_key_references_but_keeps_descriptive_text() {
        let mut r = request();
        r.assets.insert("first".into(), image());
        r.assets.insert("second".into(), image());
        r.state = "first".into();
        r.fields.get_mut("invoice").unwrap().instructions = Some("read second".into());
        assert!(encode(&r).is_err());
        r.state.clear();
        r.fields.get_mut("invoice").unwrap().instructions = Some("first".into());
        assert!(encode(&r).is_err());
        r.fields.get_mut("invoice").unwrap().instructions = Some("read first".into());
        assert_eq!(encode(&r).unwrap()["images"].as_array().unwrap().len(), 2);
        r.state = "first is the front, second is the back".into();
        assert_eq!(encode(&r).unwrap()["state"], r.state);
    }

    #[test]
    fn cloudflare_errors_do_not_echo_document_or_credentials() {
        let error = decode(
            serde_json::json!({"success":false,"errors":[{"message":"private document secret"}]}),
            &request(),
        )
        .unwrap_err()
        .to_string();
        assert!(!error.contains("private"));
        assert!(!error.contains("secret"));
    }
}
