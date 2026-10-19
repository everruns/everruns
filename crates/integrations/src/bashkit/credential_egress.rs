//! Resolve at each HTTP send; only fresh opaque references enter the interpreter.
use async_trait::async_trait;
use everruns_contracts::runtime::credential_http::{
    CredentialRedactor, binding_matches, expected_header,
};
use everruns_contracts::runtime::credential_services::*;
use everruns_contracts::runtime::resource_access::CredentialUsage;
use everruns_contracts::runtime::{
    EgressError, EgressRequest, EgressResponse, EgressResult, EgressService, EgressStreamResponse,
    SecretAuth,
};
use futures::StreamExt;
use std::sync::Arc;

pub(super) struct CredentialEgress {
    pub inner: Arc<dyn EgressService>,
    pub service: Arc<dyn CredentialService>,
    pub invocation: CredentialInvocation,
    pub usage: CredentialUsage,
    pub references: Vec<(CredentialInfo, String)>,
}
fn denied() -> EgressError {
    EgressError::InvalidRequest("Credential unavailable or request does not match its approved binding; reconnect or correct the request".into())
}

impl CredentialEgress {
    fn needs_credentials(&self, request: &EgressRequest) -> bool {
        self.references.iter().any(|(_, placeholder)| {
            request.url.contains(placeholder)
                || request.headers.values().any(|v| v.contains(placeholder))
                || request
                    .body
                    .windows(placeholder.len())
                    .any(|v| v == placeholder.as_bytes())
        })
    }
    async fn prepare(
        &self,
        mut request: EgressRequest,
    ) -> EgressResult<(EgressRequest, CredentialRedactor)> {
        let mut names = Vec::new();
        for (info, placeholder) in &self.references {
            if request.url.contains(placeholder) {
                return Err(denied());
            }
            if request.headers.values().any(|v| v.contains(placeholder))
                || request
                    .body
                    .windows(placeholder.len())
                    .any(|v| v == placeholder.as_bytes())
            {
                names.push(info.name.clone());
            }
        }
        if names.is_empty() {
            return Ok((request, CredentialRedactor::default()));
        }
        let mut headers = std::collections::HashSet::new();
        for key in request.headers.keys() {
            if !headers.insert(key.to_ascii_lowercase())
                || [
                    "host",
                    "content-length",
                    "transfer-encoding",
                    "connection",
                    "upgrade",
                    "te",
                ]
                .contains(&key.to_ascii_lowercase().as_str())
            {
                return Err(denied());
            }
        }
        let secrets = self
            .service
            .resolve(self.invocation, self.usage, names)
            .await
            .map_err(|_| denied())?;
        for secret in &secrets {
            let (original, placeholder) = self
                .references
                .iter()
                .find(|(info, _)| info.name == secret.info.name)
                .ok_or_else(denied)?;
            if original.id != secret.info.id
                || original.binding != secret.info.binding
                || !binding_matches(&secret.info.binding, &request.url)
            {
                return Err(denied());
            }
            let mut used = false;
            for (name, value) in request.headers.iter_mut() {
                if !value.contains(placeholder) {
                    continue;
                }
                let (header, prefix) =
                    expected_header(&secret.info.binding.auth).ok_or_else(denied)?;
                if !name.eq_ignore_ascii_case(header)
                    || value != &format!("{prefix}{placeholder}")
                    || secret.value.contains(['\r', '\n'])
                {
                    return Err(denied());
                }
                *value = format!("{prefix}{}", secret.value);
                used = true;
            }
            if request
                .body
                .windows(placeholder.len())
                .any(|v| v == placeholder.as_bytes())
            {
                let SecretAuth::FormField { name } = &secret.info.binding.auth else {
                    return Err(denied());
                };
                if !request.headers.iter().any(|(k, v)| {
                    k.eq_ignore_ascii_case("content-type")
                        && v.split(';').next().is_some_and(|v| {
                            v.trim()
                                .eq_ignore_ascii_case("application/x-www-form-urlencoded")
                        })
                }) {
                    return Err(denied());
                }
                let mut fields: Vec<(String, String)> = url::form_urlencoded::parse(&request.body)
                    .into_owned()
                    .collect();
                for (field, value) in &mut fields {
                    if value.contains(placeholder) {
                        if field != name || value != placeholder {
                            return Err(denied());
                        }
                        *value = secret.value.clone();
                        used = true;
                    }
                }
                let mut serializer = url::form_urlencoded::Serializer::new(String::new());
                serializer.extend_pairs(fields);
                request.body = serializer.finish().into_bytes();
            }
            if !used {
                return Err(denied());
            }
        }
        if request.headers.len() > 64
            || request.body.len() > 256 * 1024
            || request
                .headers
                .iter()
                .map(|(k, v)| k.len() + v.len())
                .sum::<usize>()
                > 64 * 1024
        {
            return Err(denied());
        }
        request
            .headers
            .retain(|k, _| !k.eq_ignore_ascii_case("accept-encoding"));
        request
            .headers
            .insert("accept-encoding".into(), "identity".into());
        let redactor = CredentialRedactor::new(
            secrets
                .iter()
                .map(|s| (s.info.name.as_str(), s.value.as_str())),
        );
        if !redactor.ready() {
            return Err(denied());
        }
        Ok((request, redactor))
    }
}

#[async_trait]
impl EgressService for CredentialEgress {
    async fn send(&self, request: EgressRequest) -> EgressResult<EgressResponse> {
        if !self.needs_credentials(&request) {
            return self.inner.send(request).await;
        }
        let (request, redactor) = self.prepare(request).await?;
        let mut response = self
            .inner
            .send_stream(request)
            .await
            .map_err(|_| denied())?;
        if response.headers.len() > 64
            || response
                .headers
                .iter()
                .map(|(k, v)| k.len() + v.len())
                .sum::<usize>()
                > 64 * 1024
            || response.headers.iter().any(|(k, v)| {
                k.eq_ignore_ascii_case("content-encoding") && !v.eq_ignore_ascii_case("identity")
            })
        {
            return Err(denied());
        }
        let mut body = Vec::new();
        while let Some(chunk) = response.body.next().await {
            let chunk = chunk.map_err(|_| denied())?;
            if body.len().saturating_add(chunk.len()) > 256 * 1024 {
                return Err(EgressError::InvalidRequest(
                    "Credential response exceeds the safe limit".into(),
                ));
            }
            body.extend_from_slice(&chunk);
        }
        let content_type = response
            .headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("content-type"))
            .map(|(_, v)| v.as_str())
            .unwrap_or_default();
        let body = redactor
            .response_body(&body, content_type)
            .map_err(|_| denied())?;
        if body.len() > 256 * 1024 {
            return Err(denied());
        }
        Ok(EgressResponse {
            status: response.status,
            headers: response
                .headers
                .into_iter()
                .filter(|(k, _)| {
                    !["content-length", "etag", "digest", "content-md5"]
                        .contains(&k.to_ascii_lowercase().as_str())
                })
                .map(|(k, v)| (redactor.text(&k), redactor.text(&v)))
                .collect(),
            body,
        })
    }
    async fn send_stream(&self, request: EgressRequest) -> EgressResult<EgressStreamResponse> {
        if !self.needs_credentials(&request) {
            return self.inner.send_stream(request).await;
        }
        let response = self.send(request).await?;
        Ok(EgressStreamResponse {
            status: response.status,
            headers: response.headers,
            body: Box::pin(futures::stream::once(async move { Ok(response.body) })),
        })
    }
    fn name(&self) -> &'static str {
        "CredentialEgress"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::credential_test_support::{Credentials, Http};
    use everruns_contracts::{
        runtime::{EgressRequestKind, SecretBinding},
        typed_id::SessionId,
    };
    use std::sync::atomic::Ordering;
    use uuid::Uuid;
    pub(super) fn fixture(auth: SecretAuth) -> (CredentialEgress, Arc<Credentials>, Arc<Http>) {
        let info = CredentialInfo {
            id: Uuid::now_v7(),
            revision: 1,
            name: "TOKEN".into(),
            binding: SecretBinding {
                url: "https://api.example/v1".into(),
                auth,
            },
            scope: everruns_contracts::runtime::resource_access::ResourceScope::Agent,
            expires_at: None,
        };
        let service = Arc::new(Credentials::default());
        service.values.lock().unwrap().push(ResolvedCredential {
            info: info.clone(),
            value: "secret +/sentinel".into(),
        });
        let mut http = Http::json("secret +/sentinel secret+%2B%2Fsentinel");
        http.headers
            .insert("content-type".into(), "text/plain".into());
        http.chunks = vec![
            b"secret +/".to_vec(),
            b"sentinel secret+%2B%2Fsentinel".to_vec(),
        ];
        http.headers
            .insert("x-echo".into(), "secret +/sentinel".into());
        let http = Arc::new(http);
        (
            CredentialEgress {
                inner: http.clone(),
                service: service.clone(),
                invocation: CredentialInvocation {
                    session_id: SessionId::new(),
                    input_message_id: Uuid::now_v7(),
                },
                usage: CredentialUsage::Service,
                references: vec![(info, "opaque_handle".into())],
            },
            service,
            http,
        )
    }
    pub(super) fn request(url: &str) -> EgressRequest {
        EgressRequest::new("GET", url, EgressRequestKind::Capability)
            .header("authorization", "Bearer opaque_handle")
    }
    #[tokio::test]
    async fn interpreter_only_receives_redacted_bytes_even_across_stream_chunks() {
        let (egress, _, http) = fixture(SecretAuth::Bearer);
        let response = egress
            .send(request("https://api.example/v1/data"))
            .await
            .unwrap();
        assert_eq!(
            http.seen.lock().unwrap()[0].headers["authorization"],
            "Bearer secret +/sentinel"
        );
        let text = String::from_utf8(response.body).unwrap();
        assert_eq!(text, "$TOKEN $TOKEN");
        assert_eq!(response.headers["x-echo"], "$TOKEN");
    }
    #[tokio::test]
    async fn revocation_blocks_next_send_in_a_long_lived_shell() {
        let (egress, service, http) = fixture(SecretAuth::Bearer);
        egress
            .send(request("https://api.example/v1"))
            .await
            .unwrap();
        service.revoked.store(true, Ordering::SeqCst);
        assert!(
            egress
                .send(request("https://api.example/v1"))
                .await
                .is_err()
        );
        assert_eq!(http.seen.lock().unwrap().len(), 1);
    }
    #[tokio::test]
    async fn wrong_origin_path_or_transport_field_never_dispatches() {
        let (egress, _, http) = fixture(SecretAuth::Bearer);
        for url in [
            "https://api.example.evil/v1",
            "https://api.example/v10",
            "https://api.example/v1/%2e%2e/admin",
            "http://api.example/v1",
        ] {
            assert!(egress.send(request(url)).await.is_err());
        }
        assert!(
            egress
                .send(
                    EgressRequest::new(
                        "GET",
                        "https://api.example/v1",
                        EgressRequestKind::Capability
                    )
                    .header("x-wrong", "opaque_handle")
                )
                .await
                .is_err()
        );
        assert!(http.seen.lock().unwrap().is_empty());
    }
    #[tokio::test]
    async fn form_substitution_happens_before_url_encoding() {
        let (egress, _, http) = fixture(SecretAuth::FormField {
            name: "assertion".into(),
        });
        let request = EgressRequest::new(
            "POST",
            "https://api.example/v1/token",
            EgressRequestKind::Capability,
        )
        .header("content-type", "application/x-www-form-urlencoded")
        .body(b"assertion=opaque_handle&grant_type=test".to_vec());
        egress.send(request).await.unwrap();
        assert_eq!(
            http.seen.lock().unwrap()[0].body,
            b"assertion=secret+%2B%2Fsentinel&grant_type=test"
        );
    }
}

#[cfg(test)]
mod unicode_echo_tests {
    use super::*;
    use crate::credential_test_support::Http;
    #[tokio::test]
    async fn json_unicode_escapes_are_redacted_before_interpreter_access() {
        let (mut egress, _, _) = super::tests::fixture(SecretAuth::Bearer);
        egress.inner = Arc::new(Http::json(r#"{"echo":"\u0073ecret +/sentinel"}"#));
        let response = egress
            .send(super::tests::request("https://api.example/v1"))
            .await
            .unwrap();
        let value: serde_json::Value = serde_json::from_slice(&response.body).unwrap();
        assert_eq!(value["echo"], "$TOKEN");
    }
}
