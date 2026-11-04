//! Credential-aware raw HTTP. No unsanitized response crosses the tool boundary.
use super::{ToolContext, ToolExecutionResult, request};
use chrono::{DateTime, Utc};
use everruns_contracts::runtime::credential_http::{
    CredentialRedactor, binding_matches, expected_header, unique_json, validate_name,
};
use everruns_contracts::runtime::credential_services::*;
use everruns_contracts::runtime::resource_access::CredentialUsage;
use everruns_contracts::runtime::{
    EgressAccess, EgressRequest, EgressRequestKind, SecretAuth, SystemEgressPolicy,
};
use everruns_contracts::tool_approval_types::{
    TOOL_APPROVAL_REQUIRED_CODE, ToolApprovalRequired, preview_tool_arguments,
};
use futures::StreamExt;
use serde_json::{Value, json};
use std::collections::BTreeSet;

const BODY_LIMIT: usize = 256 * 1024;

pub(super) fn needed(arguments: &Value) -> bool {
    arguments
        .get("capture_secrets")
        .is_some_and(|v| !v.is_null())
        || ["headers", "form", "url", "body", "json"]
            .iter()
            .any(|key| has_reference(&arguments[*key]))
}
fn has_reference(value: &Value) -> bool {
    match value {
        Value::String(s) => s
            .as_bytes()
            .windows(2)
            .any(|pair| pair[0] == b'$' && (pair[1].is_ascii_alphabetic() || pair[1] == b'_')),
        Value::Array(v) => v.iter().any(has_reference),
        Value::Object(v) => v.values().any(has_reference),
        _ => false,
    }
}
fn reference(value: &str) -> Option<&str> {
    value.strip_prefix('$').filter(|name| validate_name(name))
}
fn fail(message: &str) -> ToolExecutionResult {
    ToolExecutionResult::tool_error(message)
}

pub(super) async fn execute(
    arguments: &Value,
    context: Option<&ToolContext>,
    policy: Option<&SystemEgressPolicy>,
) -> ToolExecutionResult {
    match run(arguments, context, policy).await {
        Ok(value) => ToolExecutionResult::success(value),
        Err(message) => fail(&message),
    }
}

async fn run(
    arguments: &Value,
    context: Option<&ToolContext>,
    policy: Option<&SystemEgressPolicy>,
) -> Result<Value, String> {
    let context = context.ok_or("Credential requests require a hosted execution context")?;
    let service = context
        .storage_store
        .as_ref()
        .and_then(|s| s.credential_service())
        .ok_or("This host does not provide the credential service")?;
    let invocation = *context
        .extension::<CredentialInvocation>()
        .ok_or("Credential requests require a verified invocation")?;
    let egress = context
        .egress_service
        .as_ref()
        .ok_or("Credential requests require platform egress")?;
    for key in [
        "crawl",
        "render",
        "save_to_file",
        "content_focus",
        "selector",
        "extract",
        "max_pages",
    ] {
        if arguments
            .get(key)
            .is_some_and(|v| !v.is_null() && *v != Value::Bool(false))
        {
            return Err(
                "Credential requests cannot crawl, render, extract, or save response files".into(),
            );
        }
    }
    for key in ["url", "body", "json"] {
        if has_reference(&arguments[key]) {
            return Err(
                "Credential references are allowed only in approved headers and named form fields"
                    .into(),
            );
        }
    }
    let initial = request::parse(arguments)?;
    let initial_url =
        url::Url::parse(&initial.url).map_err(|_| "Invalid credential request URL")?;
    if initial_url.scheme() != "https"
        || !initial_url.username().is_empty()
        || initial_url.password().is_some()
        || initial_url.fragment().is_some()
    {
        return Err(
            "Credential requests require an HTTPS URL without embedded credentials or fragments"
                .into(),
        );
    }
    if let Some(policy) = policy {
        policy
            .check(
                &initial.url,
                EgressAccess::classify(&initial.method, initial.body.is_some()),
                None,
            )
            .map_err(|_| "Credential request blocked by system network policy")?;
    }
    if context
        .network_access
        .as_ref()
        .is_some_and(|acl| !acl.is_url_allowed(&initial.url))
    {
        return Err("Credential request blocked by network policy".into());
    }
    let usage: CredentialUsage = serde_json::from_value(
        arguments
            .get("credential_usage")
            .cloned()
            .unwrap_or(json!("service")),
    )
    .map_err(|_| "credential_usage must be user or service")?;
    let captures: Vec<CaptureSpec> = serde_json::from_value(
        arguments
            .get("capture_secrets")
            .cloned()
            .unwrap_or(json!([])),
    )
    .map_err(|_| "Invalid secret capture declarations")?;
    validate_captures(&captures)?;
    let mut names = BTreeSet::new();
    for key in ["headers", "form"] {
        if let Some(map) = arguments[key].as_object() {
            for value in map.values().filter_map(Value::as_str) {
                if let Some(name) = reference(value.strip_prefix("Bearer ").unwrap_or(value)) {
                    names.insert(name.to_string());
                } else if has_reference(&Value::String(value.to_string())) {
                    return Err("Use one exact $NAME reference in each credential field".into());
                }
            }
        }
    }
    let secrets = service
        .resolve(invocation, usage, names.into_iter().collect())
        .await
        .map_err(|e| e.to_string())?;
    let mut wire = arguments.clone();
    for secret in &secrets {
        if !binding_matches(&secret.info.binding, &initial.url) {
            return Err("Credential destination does not match its approved binding".into());
        }
        let placeholder = format!("${}", secret.info.name);
        let mut used = false;
        for key in ["headers", "form"] {
            if let Some(map) = wire[key].as_object_mut() {
                for (field, value) in map.iter_mut() {
                    let Some(text) = value.as_str() else { continue };
                    if text != placeholder && text != format!("Bearer {placeholder}") {
                        continue;
                    }
                    let replacement = if key == "headers" {
                        let (header, prefix) = expected_header(&secret.info.binding.auth)
                            .ok_or("This credential is bound to a form field")?;
                        if !field.eq_ignore_ascii_case(header)
                            || text != format!("{prefix}{placeholder}")
                        {
                            return Err(
                                "Credential header does not match its approved placement".into()
                            );
                        }
                        format!("{prefix}{}", secret.value)
                    } else {
                        if !matches!(&secret.info.binding.auth,SecretAuth::FormField {name} if name==field)
                            || text != placeholder
                        {
                            return Err(
                                "Credential form field does not match its approved placement"
                                    .into(),
                            );
                        }
                        secret.value.clone()
                    };
                    *value = Value::String(replacement);
                    used = true;
                }
            }
        }
        if !used {
            return Err("Credential reference was not used in an approved location".into());
        }
    }
    let request = request::parse(&wire)
        .map_err(|_| "Credential request failed validation after substitution")?;
    let mut ticket = None;
    if !captures.is_empty() {
        for spec in &captures {
            if context
                .network_access
                .as_ref()
                .is_some_and(|acl| !acl.is_url_allowed(&spec.binding.url))
            {
                return Err("Capture destination blocked by network policy".into());
            }
            if let Some(policy) = policy {
                policy
                    .check(
                        &spec.binding.url,
                        EgressAccess::classify("POST", true),
                        None,
                    )
                    .map_err(|_| "Capture destination blocked by system network policy")?;
            }
        }
        let plan = CapturePlan {
            usage,
            source_url: initial.url.clone(),
            arguments: arguments.clone(),
            captures: captures.clone(),
        };
        match service
            .prepare_capture(invocation, plan)
            .await
            .map_err(|e| e.to_string())?
        {
            CapturePreparation::Completed { result } => return Ok(result),
            CapturePreparation::Ready { ticket: id } => ticket = Some(id),
            CapturePreparation::ConsentRequired {
                ticket,
                expires_at,
                summary,
            } => {
                let (preview, truncated) = preview_tool_arguments(&summary);
                if truncated {
                    return Err("Credential consent plan is too large to display safely".into());
                }
                let marker = format!("credential:{ticket}");
                let required=ToolApprovalRequired {code:TOOL_APPROVAL_REQUIRED_CODE.into(),error:"Credential storage needs your approval. No request has been sent. After approval, retry exactly the same call.".into(),tool_call_id:context.tool_call_id.clone().ok_or("Missing tool call identity")?,tool:"web_fetch".into(),display_name:Some("Save service credentials".into()),arguments:preview,arguments_truncated:false,fingerprint:marker.clone(),risk:marker,mode:"credential_consent".into(),asked_at:Utc::now().to_rfc3339(),expires_at:expires_at.to_rfc3339()};
                return serde_json::to_value(required)
                    .map_err(|_| "Could not create credential consent request".into());
            }
        }
    }
    let mut outbound =
        EgressRequest::new(&request.method, &request.url, EgressRequestKind::Capability)
            .network_access(context.network_access.clone())
            .require_dns_pinning()
            .timeout_ms(30_000);
    for (name, value) in request.headers {
        outbound = outbound.header(name, value);
    }
    outbound = outbound.header("accept-encoding", "identity");
    if let Some(body) = request.body {
        outbound = outbound.body(body);
    }
    let mut response = egress
        .send_stream(outbound)
        .await
        .map_err(|_| "Credential HTTP request failed; the upstream outcome may be uncertain")?;
    if response.headers.iter().any(|(k, v)| {
        k.eq_ignore_ascii_case("content-encoding") && !v.eq_ignore_ascii_case("identity")
    }) {
        return Err("Encoded credential response withheld".into());
    }
    if response.headers.len() > 64
        || response
            .headers
            .iter()
            .map(|(k, v)| k.len() + v.len())
            .sum::<usize>()
            > 64 * 1024
    {
        return Err("Credential response headers exceed the limit".into());
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.body.next().await {
        let chunk = chunk.map_err(|_| "Credential response could not be read safely")?;
        if body.len().saturating_add(chunk.len()) > BODY_LIMIT {
            return Err("Credential response exceeds the limit; response withheld".into());
        }
        body.extend_from_slice(&chunk);
    }
    let mut captured = Vec::new();
    let mut text = String::from_utf8(body)
        .map_err(|_| "Credential response is not valid UTF-8; response withheld")?;
    if !captures.is_empty() {
        if !(200..300).contains(&response.status) {
            let json = unique_json(text.as_bytes())
                .map_err(|_| "Authentication failed; upstream response withheld")?;
            let code = json.get("error").and_then(Value::as_str).filter(|s| {
                matches!(
                    *s,
                    "authorization_pending"
                        | "slow_down"
                        | "access_denied"
                        | "expired_token"
                        | "invalid_grant"
                        | "invalid_client"
                )
            });
            let code = code.ok_or("Authentication failed; upstream response withheld")?;
            let result=CredentialRedactor::new(secrets.iter().map(|s|(s.info.name.as_str(),s.value.as_str()))).value(json!({"url":request.url,"method":request.method,"status_code":response.status,"error_code":code}));
            service
                .commit_capture(
                    invocation,
                    ticket.ok_or("Missing capture ticket")?,
                    Vec::new(),
                    result.clone(),
                )
                .await
                .map_err(|_| "Could not safely record authentication status")?;
            return Ok(result);
        }
        let mut json = unique_json(text.as_bytes())
            .map_err(|_| "Capture requires complete, unambiguous JSON; response withheld")?;
        let received = Utc::now();
        for spec in &captures {
            let value = json
                .pointer(&spec.from)
                .and_then(Value::as_str)
                .filter(|v| !v.is_empty() && v.len() <= 64 * 1024)
                .ok_or("A configured secret field is missing or invalid; response withheld")?
                .to_string();
            let expires_at = expiry(spec, &json, received)?;
            captured.push(CapturedCredential {
                name: spec.name.clone(),
                value,
                expires_at,
            });
        }
        for spec in &captures {
            *json
                .pointer_mut(&spec.from)
                .ok_or("Invalid capture pointer")? = json!(format!("${}", spec.name));
        }
        text = serde_json::to_string(&json).map_err(|_| "Could not sanitize captured JSON")?;
    }
    let redactor = CredentialRedactor::new(
        secrets
            .iter()
            .map(|s| (s.info.name.as_str(), s.value.as_str()))
            .chain(captured.iter().map(|s| (s.name.as_str(), s.value.as_str()))),
    );
    if !redactor.ready() {
        return Err("Credential response could not be sanitized; response withheld".into());
    }

    let content_type = response
        .headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("content-type"))
        .map(|(_, v)| v.as_str())
        .unwrap_or_default();
    let body = String::from_utf8(
        redactor
            .response_body(text.as_bytes(), content_type)
            .map_err(|_| "Credential response could not be sanitized; response withheld")?,
    )
    .map_err(|_| "Credential response is not UTF-8; response withheld")?;
    let result = json!({"url":request.url,"method":request.method,"status_code":response.status,"headers":redactor.value(json!(response.headers)),"body":body,"size":body.len(),"saved_credentials":captured.iter().map(|s|json!({"name":s.name,"reference":format!("${}",s.name),"expires_at":s.expires_at})).collect::<Vec<_>>()});
    if serde_json::to_vec(&result)
        .map_err(|_| "Credential response could not be sanitized")?
        .len()
        > 512 * 1024
    {
        return Err("Sanitized credential response exceeds the limit; response withheld".into());
    }
    if let Some(ticket) = ticket {
        service
            .commit_capture(invocation, ticket, captured, result.clone())
            .await
            .map_err(|_| "Credential storage failed or authorization changed; response withheld")?;
    }
    Ok(result)
}

fn validate_captures(captures: &[CaptureSpec]) -> Result<(), String> {
    if captures.len() > 8 {
        return Err("At most eight credentials can be captured".into());
    }
    let mut names = BTreeSet::new();
    for (i, spec) in captures.iter().enumerate() {
        if !validate_name(&spec.name)
            || !names.insert(&spec.name)
            || spec.binding.validate().is_err()
        {
            return Err("Invalid or duplicate capture name/binding".into());
        }
        if spec.expires_in_from.is_some() && spec.expires_at_from.is_some() {
            return Err("Choose one expiry format".into());
        }
        for pointer in std::iter::once(&spec.from)
            .chain(spec.expires_in_from.iter())
            .chain(spec.expires_at_from.iter())
        {
            if !pointer.starts_with('/') || pointer.len() > 1024 || pointer.split('/').count() > 18
            {
                return Err("Invalid capture JSON pointer".into());
            }
            let mut chars = pointer.chars();
            while let Some(c) = chars.next() {
                if c == '~' && !matches!(chars.next(), Some('0' | '1')) {
                    return Err("Invalid capture JSON pointer escape".into());
                }
            }
        }
        for other in &captures[..i] {
            if spec.from == other.from
                || spec.from.starts_with(&format!("{}/", other.from))
                || other.from.starts_with(&format!("{}/", spec.from))
            {
                return Err("Capture pointers must not overlap".into());
            }
        }
    }
    Ok(())
}

fn expiry(
    spec: &CaptureSpec,
    json: &Value,
    received: DateTime<Utc>,
) -> Result<Option<DateTime<Utc>>, String> {
    let result = if let Some(pointer) = &spec.expires_in_from {
        let seconds = json
            .pointer(pointer)
            .and_then(Value::as_i64)
            .filter(|v| *v > 0)
            .ok_or("Invalid relative credential expiry")?;
        Some(
            received
                .checked_add_signed(
                    chrono::Duration::try_seconds(seconds).ok_or("Credential expiry overflow")?,
                )
                .ok_or("Credential expiry overflow")?,
        )
    } else if let Some(pointer) = &spec.expires_at_from {
        Some(
            DateTime::parse_from_rfc3339(
                json.pointer(pointer)
                    .and_then(Value::as_str)
                    .ok_or("Invalid absolute credential expiry")?,
            )
            .map_err(|_| "Invalid absolute credential expiry")?
            .with_timezone(&Utc),
        )
    } else {
        None
    };
    if result.is_some_and(|e| e <= received) {
        return Err("Captured credential is already expired".into());
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn duplicate_keys_are_never_captured_or_displayed() {
        assert!(unique_json(br#"{"access_token":"first","access_token":"second"}"#).is_err());
        assert!(unique_json(br#"{"outer":{"token":"a","token":"b"}}"#).is_err());
    }
    #[test]
    fn capture_get_selects_secure_raw_path() {
        assert!(needed(
            &json!({"url":"https://example.com","capture_secrets":[]})
        ));
        assert!(needed(
            &json!({"headers":{"Authorization":"Bearer $TOKEN"}})
        ));
        assert!(!needed(&json!({"url":"https://example.com"})));
    }
}
