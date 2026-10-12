// Poppy discovery (spec 3): the channel's `poppy.json`, which the company's
// own `/.well-known/poppy.json` redirects to, and its OAuth server metadata
// (RFC 8414) with `poppy_domains`.
//
// Design Decisions:
// - `organization.domain` must match the host the personal agent asked (spec
//   3.1), and a company may have several domains, so the redirect target
//   names the domain: `poppy.json?domain=example.co.uk`. Without `domain` the
//   channel's first domain is used; a domain the channel does not list is a
//   `404`, so the document never speaks for a domain the company did not
//   configure.
// - `auth` lists no sign-in types yet: only signed-out Sessions (spec 3.1).

use axum::{
    extract::{OriginalUri, Path, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use serde_json::{Value, json};

use super::{Channel, PoppyState, Urls, not_found};

/// Discovery documents are public and change only with the channel config.
const CACHE_CONTROL: &str = "public, max-age=300";

#[derive(Debug, Deserialize)]
pub(super) struct DomainQuery {
    domain: Option<String>,
}

/// `GET {base}/poppy.json`.
pub(super) async fn poppy_json(
    State(state): State<PoppyState>,
    Path(channel_id): Path<String>,
    Query(query): Query<DomainQuery>,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
) -> Response {
    let channel = match super::channel(&state, &channel_id).await {
        Ok(channel) => channel,
        Err(response) => return response,
    };
    let domain = match query.domain.as_deref() {
        Some(domain) => {
            let domain = domain.trim_start_matches("www.");
            match channel.config.domains.iter().find(|known| *known == domain) {
                Some(domain) => domain.clone(),
                None => return not_found(),
            }
        }
        None => channel.config.domains[0].clone(),
    };
    let urls = Urls::from_request(&headers, uri.path(), &channel_id);
    public(document(&channel, &urls, &domain))
}

/// Pure: the `poppy.json` for `domain`.
fn document(channel: &Channel, urls: &Urls, domain: &str) -> Value {
    let name = channel
        .config
        .organization_name
        .clone()
        .unwrap_or_else(|| channel.context.name.clone());
    json!({
        "protocol_version": "0.1",
        "organization": { "name": name, "domain": domain },
        "auth": { "issuer": urls.issuer },
        "agent": {
            "protocols": [{ "type": "poppy", "endpoint": urls.conversations() }]
        },
    })
}

/// `GET {base}/.well-known/oauth-authorization-server`.
pub(super) async fn metadata(
    State(state): State<PoppyState>,
    Path(channel_id): Path<String>,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
) -> Response {
    let urls = Urls::from_request(&headers, uri.path(), &channel_id);
    serve_metadata(&state, &channel_id, &urls).await
}

/// `GET /.well-known/oauth-authorization-server/{issuer path}`: RFC 8414 §3.1
/// for an issuer with a path. Only paths ending in a channel's Poppy base.
pub(super) async fn root_metadata(
    State(state): State<PoppyState>,
    Path(issuer_path): Path<String>,
    headers: HeaderMap,
) -> Response {
    let Some(channel_id) = channel_of_issuer_path(&issuer_path) else {
        return not_found();
    };
    let urls = Urls::from_request(&headers, &format!("/{issuer_path}"), channel_id);
    serve_metadata(&state, channel_id, &urls).await
}

/// Pure: the channel id in an issuer path `{prefix}/v1/channels/{id}/poppy`.
fn channel_of_issuer_path(path: &str) -> Option<&str> {
    let rest = path.strip_suffix("/poppy")?;
    let (head, channel_id) = rest.rsplit_once('/')?;
    (head == "v1/channels" || head.ends_with("/v1/channels"))
        .then_some(channel_id)
        .filter(|id| !id.is_empty())
}

async fn serve_metadata(state: &PoppyState, channel_id: &str, urls: &Urls) -> Response {
    let channel = match super::channel(state, channel_id).await {
        Ok(channel) => channel,
        Err(response) => return response,
    };
    public(metadata_document(&channel, urls))
}

/// Pure: the RFC 8414 metadata.
fn metadata_document(channel: &Channel, urls: &Urls) -> Value {
    let algs = ["ES256", "ES384", "RS256", "PS256", "EdDSA"];
    json!({
        "issuer": urls.issuer,
        "token_endpoint": urls.token(),
        "revocation_endpoint": urls.revocation(),
        "grant_types_supported": ["urn:ietf:params:oauth:grant-type:jwt-bearer"],
        "token_endpoint_auth_methods_supported": ["private_key_jwt"],
        "token_endpoint_auth_signing_alg_values_supported": algs,
        "revocation_endpoint_auth_methods_supported": ["private_key_jwt"],
        "dpop_signing_alg_values_supported": algs,
        "client_id_metadata_document_supported": true,
        "scopes_supported": [],
        "poppy_domains": channel.config.domains,
    })
}

fn public(body: Value) -> Response {
    (
        StatusCode::OK,
        [
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/json"),
            ),
            (
                header::CACHE_CONTROL,
                HeaderValue::from_static(CACHE_CONTROL),
            ),
        ],
        body.to_string(),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_channel_in_an_issuer_path() {
        assert_eq!(
            channel_of_issuer_path("api/v1/channels/ch_1/poppy"),
            Some("ch_1")
        );
        assert_eq!(
            channel_of_issuer_path("v1/channels/ch_1/poppy"),
            Some("ch_1")
        );
        assert_eq!(channel_of_issuer_path("v1/channels/ch_1/a2a"), None);
        assert_eq!(channel_of_issuer_path("v1/agents/ch_1/poppy"), None);
        assert_eq!(channel_of_issuer_path("v1/channels//poppy"), None);
    }
}
