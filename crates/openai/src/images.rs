use anyhow::{Context, Result, anyhow};
use reqwest::multipart::{Form, Part};
use reqwest::{Client, RequestBuilder, Url};
use serde::{Deserialize, Serialize};

const DEFAULT_OPENAI_BASE_URL: &str = "https://api.openai.com/v1";

#[derive(Debug, Clone)]
pub struct OpenAiImageClient {
    client: Client,
    api_key: String,
    base_url: Option<String>,
}

impl OpenAiImageClient {
    pub fn new(api_key: impl Into<String>, base_url: Option<String>) -> Result<Self> {
        Ok(Self {
            client: Client::builder()
                .timeout(std::time::Duration::from_secs(120))
                .build()
                .context("failed to build OpenAI image client")?,
            api_key: api_key.into(),
            base_url,
        })
    }

    pub async fn generate(&self, request: GenerateImageRequest) -> Result<ImageApiResponse> {
        let url = image_endpoint_url(self.base_url.as_deref(), "images/generations")?;
        let response = self
            .apply_auth(self.client.post(url.clone()), url.as_str())
            .json(&request)
            .send()
            .await
            .context("failed to call OpenAI image generation API")?;

        parse_image_response(response).await
    }

    pub async fn edit(&self, request: EditImageRequest) -> Result<ImageApiResponse> {
        let url = image_endpoint_url(self.base_url.as_deref(), "images/edits")?;
        let image_field_name = if request.images.len() > 1 {
            "image[]"
        } else {
            "image"
        };

        let mut form = Form::new()
            .text("model", request.model)
            .text("prompt", request.prompt)
            .text("n", request.count.to_string());

        if let Some(size) = request.size {
            form = form.text("size", size);
        }
        if let Some(quality) = request.quality {
            form = form.text("quality", quality);
        }
        if let Some(background) = request.background {
            form = form.text("background", background);
        }
        if let Some(output_format) = request.output_format {
            form = form.text("output_format", output_format);
        }

        for image in request.images {
            let part = Part::bytes(image.data)
                .file_name(image.filename)
                .mime_str(&image.content_type)
                .context("invalid source image content type")?;
            form = form.part(image_field_name.to_string(), part);
        }

        let response = self
            .apply_auth(self.client.post(url.clone()), url.as_str())
            .multipart(form)
            .send()
            .await
            .context("failed to call OpenAI image edit API")?;

        parse_image_response(response).await
    }

    fn apply_auth(&self, request: RequestBuilder, api_url: &str) -> RequestBuilder {
        if everruns_core::openai_protocol::is_azure_openai_api_url(api_url) {
            request.header("api-key", &self.api_key)
        } else {
            request.bearer_auth(&self.api_key)
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct GenerateImageRequest {
    pub model: String,
    pub prompt: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quality: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub background: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "output_format")]
    pub output_format: Option<String>,
    #[serde(rename = "n")]
    pub count: usize,
}

#[derive(Debug, Clone)]
pub struct EditImageRequest {
    pub model: String,
    pub prompt: String,
    pub images: Vec<EditImageInput>,
    pub size: Option<String>,
    pub quality: Option<String>,
    pub background: Option<String>,
    pub output_format: Option<String>,
    pub count: usize,
}

#[derive(Debug, Clone)]
pub struct EditImageInput {
    pub filename: String,
    pub content_type: String,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ImageApiResponse {
    pub data: Vec<ImageApiImage>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ImageApiImage {
    pub b64_json: String,
    #[serde(default)]
    pub revised_prompt: Option<String>,
}

fn image_endpoint_url(base_url: Option<&str>, endpoint: &str) -> Result<Url> {
    let mut normalized = base_url
        .unwrap_or(DEFAULT_OPENAI_BASE_URL)
        .trim_end_matches('/');

    for suffix in [
        "/responses",
        "/chat/completions",
        "/images/generations",
        "/images/edits",
    ] {
        if let Some(prefix) = normalized.strip_suffix(suffix) {
            normalized = prefix;
            break;
        }
    }

    Url::parse(&format!(
        "{}/{}",
        normalized,
        endpoint.trim_start_matches('/')
    ))
    .map_err(|error| anyhow!("invalid OpenAI image API URL: {error}"))
}

async fn parse_image_response(response: reqwest::Response) -> Result<ImageApiResponse> {
    let status = response.status();
    if !status.is_success() {
        let body = response
            .text()
            .await
            .unwrap_or_else(|_| "<unreadable response body>".to_string());
        return Err(anyhow!("OpenAI image API returned {status}: {body}"));
    }

    response
        .json::<ImageApiResponse>()
        .await
        .context("failed to decode OpenAI image API response")
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{body_json, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn generate_uses_bearer_auth_and_json_payload() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/images/generations"))
            .and(header("authorization", "Bearer sk-test"))
            .and(body_json(serde_json::json!({
                "model": "gpt-image-1",
                "prompt": "otter",
                "size": "1024x1024",
                "quality": "high",
                "background": "transparent",
                "output_format": "png",
                "n": 1
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": [{"b64_json": "aGVsbG8=", "revised_prompt": "otter"}]
            })))
            .mount(&server)
            .await;

        let client =
            OpenAiImageClient::new("sk-test", Some(format!("{}/v1", server.uri()))).unwrap();
        let response = client
            .generate(GenerateImageRequest {
                model: "gpt-image-1".to_string(),
                prompt: "otter".to_string(),
                size: Some("1024x1024".to_string()),
                quality: Some("high".to_string()),
                background: Some("transparent".to_string()),
                output_format: Some("png".to_string()),
                count: 1,
            })
            .await
            .unwrap();

        assert_eq!(response.data.len(), 1);
        assert_eq!(response.data[0].b64_json, "aGVsbG8=");
    }

    #[tokio::test]
    async fn edit_uses_multipart_endpoint_with_custom_base_url() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/openai/v1/images/edits"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": [{"b64_json": "aGVsbG8="}]
            })))
            .mount(&server)
            .await;

        let client =
            OpenAiImageClient::new("azure-key", Some(format!("{}/openai/v1", server.uri())))
                .unwrap();

        let response = client
            .edit(EditImageRequest {
                model: "gpt-image-1".to_string(),
                prompt: "edit it".to_string(),
                images: vec![EditImageInput {
                    filename: "source.png".to_string(),
                    content_type: "image/png".to_string(),
                    data: vec![1, 2, 3],
                }],
                size: Some("1024x1024".to_string()),
                quality: Some("medium".to_string()),
                background: Some("opaque".to_string()),
                output_format: Some("png".to_string()),
                count: 1,
            })
            .await
            .unwrap();

        assert_eq!(response.data.len(), 1);
    }
}
