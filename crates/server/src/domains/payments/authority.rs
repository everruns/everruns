// Machine-payment execution authority for server-backed tools.
//
// Decision: paid transport is internal infrastructure. Capabilities submit typed
// requests; this service resolves a wallet/policy, runs the selected native
// payment rail adapter, and records an auditable attempt.

use crate::storage::StorageBackend;
use crate::storage::encryption::EncryptionService;
use crate::storage::models::{
    CreatePaymentAttemptRow, PaymentAccountRow, PaymentAttemptRow, PaymentPolicyRow,
};
use async_trait::async_trait;
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use super::eip712::{self, Domain, LocalWallet, TransferWithAuthorization};
use everruns_core::payment::{
    MachinePaymentRequest, MachinePaymentResponse, PaymentMethod, PaymentRail,
};
use everruns_core::tool_execution::PaymentAuthority;
use everruns_provider::error::{AgentLoopError, Result};
use everruns_provider::typed_id::{AgentId, PaymentAttemptId, SessionId};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use url::Url;

#[derive(Clone)]
pub struct ServerPaymentAuthority {
    db: Arc<StorageBackend>,
    encryption: Option<Arc<EncryptionService>>,
    org_id: i64,
    agent_id: Option<AgentId>,
}

impl ServerPaymentAuthority {
    pub fn new(
        db: Arc<StorageBackend>,
        encryption: Option<Arc<EncryptionService>>,
        org_id: i64,
        agent_id: Option<AgentId>,
    ) -> Self {
        Self {
            db,
            encryption,
            org_id,
            agent_id,
        }
    }
}

struct SelectedPolicy {
    account: PaymentAccountRow,
    policy: PaymentPolicyRow,
    rail: PaymentRail,
}

#[async_trait]
impl PaymentAuthority for ServerPaymentAuthority {
    async fn execute_machine_payment(
        &self,
        session_id: SessionId,
        request: MachinePaymentRequest,
    ) -> Result<MachinePaymentResponse> {
        validate_request_shape(&request)?;
        let request_hash = request_hash(&request);
        let selected = match self.select_policy(session_id, &request).await {
            Ok(selected) => selected,
            Err(error) => {
                let _ = self
                    .record_attempt(
                        &request,
                        AttemptRecord {
                            session_id,
                            payment_account_id: None,
                            rail: None,
                            request_hash: request_hash.clone(),
                            status: "failed".to_string(),
                            error_message: Some(error.to_string()),
                            receipt: json!({}),
                        },
                    )
                    .await;
                return Err(error);
            }
        };

        let result = self
            .execute_with_rail(&request, &selected.account, selected.rail.clone())
            .await;
        match result {
            Ok(response) => {
                let receipt = json!({
                    "rail": selected.rail.as_wire(),
                    "payment_account_id": selected.account.id,
                    "payment_policy_id": selected.policy.id,
                    "request_hash": request_hash.clone(),
                    "raw": response.raw_receipt,
                });
                let attempt = self
                    .record_attempt(
                        &request,
                        AttemptRecord {
                            session_id,
                            payment_account_id: Some(selected.account.id),
                            rail: Some(selected.rail.as_wire().to_string()),
                            request_hash: request_hash.clone(),
                            status: "succeeded".to_string(),
                            error_message: None,
                            receipt: receipt.clone(),
                        },
                    )
                    .await?;
                Ok(MachinePaymentResponse {
                    attempt_id: Some(PaymentAttemptId::from_uuid(attempt.id)),
                    amount_usd: request.max_amount_usd,
                    rail: Some(selected.rail),
                    response: response.body,
                    receipt,
                })
            }
            Err(error) => {
                let receipt = json!({
                    "rail": selected.rail.as_wire(),
                    "payment_account_id": selected.account.id,
                    "payment_policy_id": selected.policy.id,
                    "request_hash": request_hash.clone(),
                });
                let _ = self
                    .record_attempt(
                        &request,
                        AttemptRecord {
                            session_id,
                            payment_account_id: Some(selected.account.id),
                            rail: Some(selected.rail.as_wire().to_string()),
                            request_hash,
                            status: "failed".to_string(),
                            error_message: Some(error.to_string()),
                            receipt,
                        },
                    )
                    .await;
                Err(error)
            }
        }
    }
}

struct RailResponse {
    body: serde_json::Value,
    raw_receipt: serde_json::Value,
}

struct PaidHttpResponse {
    status: reqwest::StatusCode,
    headers: reqwest::header::HeaderMap,
    body: serde_json::Value,
}

struct AttemptRecord {
    session_id: SessionId,
    payment_account_id: Option<uuid::Uuid>,
    rail: Option<String>,
    request_hash: String,
    status: String,
    error_message: Option<String>,
    receipt: serde_json::Value,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct X402PaymentRequired {
    x402_version: i64,
    #[serde(default)]
    resource: Option<X402ResourceInfo>,
    accepts: Vec<X402PaymentRequirements>,
    #[serde(default)]
    extensions: Option<serde_json::Value>,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct X402ResourceInfo {
    url: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    mime_type: Option<String>,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct X402PaymentRequirements {
    scheme: String,
    network: String,
    asset: String,
    amount: String,
    pay_to: String,
    max_timeout_seconds: i64,
    #[serde(default)]
    extra: serde_json::Value,
}

impl ServerPaymentAuthority {
    async fn select_policy(
        &self,
        session_id: SessionId,
        request: &MachinePaymentRequest,
    ) -> Result<SelectedPolicy> {
        let session = self
            .db
            .get_session(self.org_id, session_id)
            .await
            .map_err(|error| {
                AgentLoopError::store(format!("Failed to load payment session: {error}"))
            })?
            .ok_or_else(|| AgentLoopError::session_not_found(session_id))?;

        let host = request_host(&request.url)?;
        let candidates = subject_candidates(
            session_id,
            self.agent_id.or(session.agent_id),
            session.agent_identity_id,
            session.resolved_owner_user_id,
        );
        let mut policies = Vec::new();
        let mut seen_policy_ids = HashSet::new();
        for (subject_type, subject_id) in &candidates {
            let rows = self
                .db
                .list_payment_policies(
                    self.org_id,
                    None,
                    Some(*subject_type),
                    Some(subject_id.as_str()),
                )
                .await
                .map_err(|error| {
                    AgentLoopError::store(format!("Failed to load payment policies: {error}"))
                })?;
            for row in rows {
                if seen_policy_ids.insert(row.id) {
                    policies.push(row);
                }
            }
        }

        for policy in policies {
            if policy.status != "active" {
                continue;
            }
            // THREAT[TM-AGENT-022]: Prompt-injected agents could try to spend from any wallet.
            // Mitigation: every paid capability request must match an active policy for the
            // session, agent, agent identity, user, or organization plus capability, host,
            // rail, and per-request limit before a payment is signed.
            if !candidates
                .iter()
                .any(|(kind, id)| policy.subject_type == *kind && policy.subject_id == *id)
            {
                continue;
            }
            if !list_allows(&policy.allowed_capabilities, &request.capability) {
                continue;
            }
            if !host_allowed(&policy.allowed_hosts, &host) {
                continue;
            }
            if let Some(max) = policy.max_amount_usd_per_request
                && request.max_amount_usd > max
            {
                continue;
            }

            let account = self
                .db
                .get_payment_account(self.org_id, policy.payment_account_id)
                .await
                .map_err(|error| {
                    AgentLoopError::store(format!("Failed to load payment account: {error}"))
                })?
                .ok_or_else(|| {
                    AgentLoopError::config("Payment policy points to a missing wallet")
                })?;
            if account.status != "active" {
                continue;
            }
            let account_rail = PaymentRail::from(account.rail.as_str());
            if !request.rail_preference.is_empty()
                && !request
                    .rail_preference
                    .iter()
                    .any(|rail| rail == &account_rail)
            {
                continue;
            }
            if !policy.rail_preference.is_empty()
                && !policy
                    .rail_preference
                    .iter()
                    .any(|rail| PaymentRail::from(rail.as_str()) == account_rail)
            {
                continue;
            }

            return Ok(SelectedPolicy {
                account,
                policy,
                rail: account_rail,
            });
        }

        Err(AgentLoopError::config(
            "No active payment policy allows this paid capability request",
        ))
    }

    async fn execute_with_rail(
        &self,
        request: &MachinePaymentRequest,
        account: &PaymentAccountRow,
        rail: PaymentRail,
    ) -> Result<RailResponse> {
        match rail {
            PaymentRail::X402Base => self.execute_x402_base(request, account).await,
            PaymentRail::MppTempo => Err(AgentLoopError::config(
                "Native mpp_tempo payment rail adapter is not configured",
            )),
        }
    }

    async fn execute_x402_base(
        &self,
        request: &MachinePaymentRequest,
        account: &PaymentAccountRow,
    ) -> Result<RailResponse> {
        let private_key = self.decrypt_wallet_private_key(account)?;
        let wallet = LocalWallet::from_private_key_hex(private_key.trim())
            .map_err(|error| AgentLoopError::config(format!("Invalid x402 wallet key: {error}")))?;
        let first = self.send_http_request(request, None).await?;
        if first.status != reqwest::StatusCode::PAYMENT_REQUIRED {
            if !first.status.is_success() {
                return Err(AgentLoopError::tool(format!(
                    "x402 unpaid probe failed with status {}: {}",
                    first.status, first.body
                )));
            }
            return Ok(RailResponse {
                body: first.body,
                raw_receipt: json!({
                    "protocol": "x402",
                    "payment_required": false,
                    "status": first.status.as_u16(),
                }),
            });
        }

        let required = parse_x402_payment_required(&first.headers, &first.body)?;
        let selected = select_x402_requirement(&required, request.max_amount_usd)?;
        let signature_header = create_x402_payment_signature(&wallet, &required, &selected).await?;
        let second = self
            .send_http_request(
                request,
                Some(("PAYMENT-SIGNATURE", signature_header.as_str())),
            )
            .await?;
        if !second.status.is_success() {
            return Err(AgentLoopError::tool(format!(
                "x402 paid request failed with status {}: {}",
                second.status, second.body
            )));
        }

        Ok(RailResponse {
            body: second.body,
            raw_receipt: json!({
                "protocol": "x402",
                "payment_required": true,
                "payment_response": parse_x402_payment_response(&second.headers),
                "status": second.status.as_u16(),
                "selected": selected,
            }),
        })
    }

    fn decrypt_wallet_private_key(&self, account: &PaymentAccountRow) -> Result<String> {
        let encrypted = account.credential_encrypted.as_deref().ok_or_else(|| {
            AgentLoopError::config("x402 wallet key is not configured for this payment account")
        })?;
        let encryption = self.encryption.as_ref().ok_or_else(|| {
            AgentLoopError::config("Encryption is not configured for x402 wallet custody")
        })?;
        // THREAT[TM-CRYPTO-008]: Wallet keys are sensitive signing credentials.
        // Mitigation: decrypt only inside the server payment authority immediately before
        // native signing; workers receive no key material and all attempts are recorded.
        encryption.decrypt_to_string(encrypted).map_err(|error| {
            AgentLoopError::config(format!("Failed to decrypt x402 wallet key: {error}"))
        })
    }

    async fn send_http_request(
        &self,
        request: &MachinePaymentRequest,
        payment_header: Option<(&str, &str)>,
    ) -> Result<PaidHttpResponse> {
        let client = build_payment_http_client()?;
        let mut builder = match request.method {
            PaymentMethod::Get => client.get(&request.url),
            PaymentMethod::Post => client.post(&request.url),
        };
        if let Some(body) = request.body.as_ref()
            && matches!(request.method, PaymentMethod::Post)
        {
            builder = builder.json(body);
        }
        if let Some((name, value)) = payment_header {
            builder = builder.header(name, value);
        }

        let response = builder
            .send()
            .await
            .map_err(|error| AgentLoopError::tool(format!("x402 HTTP request failed: {error}")))?;
        let status = response.status();
        let headers = response.headers().clone();
        let text = response.text().await.map_err(|error| {
            AgentLoopError::tool(format!("Failed to read x402 response: {error}"))
        })?;
        let body = serde_json::from_str(&text).unwrap_or_else(|_| json!({ "raw": text }));
        Ok(PaidHttpResponse {
            status,
            headers,
            body,
        })
    }

    async fn record_attempt(
        &self,
        request: &MachinePaymentRequest,
        attempt: AttemptRecord,
    ) -> Result<PaymentAttemptRow> {
        self.db
            .create_payment_attempt(
                self.org_id,
                CreatePaymentAttemptRow {
                    payment_account_id: attempt.payment_account_id,
                    session_id: Some(attempt.session_id.uuid()),
                    capability: request.capability.clone(),
                    operation: request.operation.clone(),
                    rail: attempt.rail,
                    amount_usd: request.max_amount_usd,
                    currency: "USD".to_string(),
                    target_url: request.url.clone(),
                    request_hash: Some(attempt.request_hash),
                    status: attempt.status,
                    error_message: attempt.error_message,
                    receipt: attempt.receipt,
                },
            )
            .await
            .map_err(|error| {
                AgentLoopError::store(format!("Failed to record payment attempt: {error}"))
            })
    }
}

fn build_payment_http_client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        // THREAT[TM-AGENT-023]: Paid endpoints may redirect to disallowed/internal hosts.
        // Mitigation: never follow redirects for machine payments; policy checks apply
        // only to the original validated URL.
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|error| {
            AgentLoopError::tool(format!(
                "Failed to initialize payment authority HTTP client: {error}"
            ))
        })
}

fn validate_request_shape(request: &MachinePaymentRequest) -> Result<()> {
    if request.max_amount_usd <= 0.0 {
        return Err(AgentLoopError::config("Payment amount must be positive"));
    }
    let parsed = Url::parse(&request.url)
        .map_err(|error| AgentLoopError::config(format!("Invalid payment URL: {error}")))?;
    if parsed.scheme() != "https" {
        return Err(AgentLoopError::config("Paid requests must use HTTPS"));
    }
    Ok(())
}

fn request_host(url: &str) -> Result<String> {
    Url::parse(url)
        .ok()
        .and_then(|url| url.host_str().map(str::to_ascii_lowercase))
        .ok_or_else(|| AgentLoopError::config("Paid request URL has no host"))
}

fn subject_candidates(
    session_id: SessionId,
    agent_id: Option<AgentId>,
    agent_identity_id: Option<everruns_provider::typed_id::AgentIdentityId>,
    user_id: Option<uuid::Uuid>,
) -> Vec<(&'static str, String)> {
    let mut candidates = vec![
        ("session", session_id.to_string()),
        ("session", session_id.uuid().to_string()),
        ("org", "org".to_string()),
    ];
    if let Some(agent_id) = agent_id {
        candidates.push(("agent", agent_id.to_string()));
        candidates.push(("agent", agent_id.uuid().to_string()));
    }
    if let Some(agent_identity_id) = agent_identity_id {
        candidates.push(("agent_identity", agent_identity_id.to_string()));
        candidates.push(("agent_identity", agent_identity_id.uuid().to_string()));
    }
    if let Some(user_id) = user_id {
        candidates.push(("user", user_id.to_string()));
    }
    candidates
}

fn list_allows(allowed: &[String], value: &str) -> bool {
    allowed.is_empty() || allowed.iter().any(|item| item == "*" || item == value)
}

fn host_allowed(allowed_hosts: &[String], host: &str) -> bool {
    allowed_hosts.is_empty()
        || allowed_hosts.iter().any(|allowed| {
            let allowed = allowed.to_ascii_lowercase();
            allowed == "*"
                || allowed == host
                || (allowed.starts_with('.') && host.ends_with(&allowed))
        })
}

fn method_wire(method: &PaymentMethod) -> &'static str {
    method.as_wire()
}

fn parse_x402_payment_required(
    headers: &reqwest::header::HeaderMap,
    body: &serde_json::Value,
) -> Result<X402PaymentRequired> {
    if let Some(header) = headers.get("PAYMENT-REQUIRED") {
        let header = header.to_str().map_err(|error| {
            AgentLoopError::tool(format!("Invalid PAYMENT-REQUIRED header: {error}"))
        })?;
        let bytes = BASE64_STANDARD.decode(header).map_err(|error| {
            AgentLoopError::tool(format!(
                "PAYMENT-REQUIRED header is not valid base64: {error}"
            ))
        })?;
        serde_json::from_slice(&bytes).map_err(|error| {
            AgentLoopError::tool(format!(
                "PAYMENT-REQUIRED header is not valid JSON: {error}"
            ))
        })
    } else {
        serde_json::from_value(body.clone()).map_err(|error| {
            AgentLoopError::tool(format!(
                "x402 payment-required body is not valid JSON: {error}"
            ))
        })
    }
}

fn select_x402_requirement(
    required: &X402PaymentRequired,
    max_amount_usd: f64,
) -> Result<X402PaymentRequirements> {
    if required.x402_version != 2 {
        return Err(AgentLoopError::tool(format!(
            "Unsupported x402 version: {}",
            required.x402_version
        )));
    }
    let selected = required
        .accepts
        .iter()
        .find(|requirement| {
            requirement.scheme == "exact"
                && requirement.network == "eip155:8453"
                && asset_transfer_method(requirement).is_none_or(|method| method == "eip3009")
        })
        .ok_or_else(|| {
            AgentLoopError::tool("No supported x402 exact/eip3009 Base requirement was offered")
        })?;
    let amount_usd = atomic_usdc_to_usd(&selected.amount)?;
    if amount_usd > max_amount_usd + 0.000_001 {
        return Err(AgentLoopError::tool(format!(
            "x402 requested ${amount_usd:.6}, above allowed ${max_amount_usd:.6}"
        )));
    }
    Ok(selected.clone())
}

fn asset_transfer_method(requirement: &X402PaymentRequirements) -> Option<&str> {
    requirement
        .extra
        .get("assetTransferMethod")
        .and_then(serde_json::Value::as_str)
}

fn atomic_usdc_to_usd(amount: &str) -> Result<f64> {
    let value = amount
        .parse::<f64>()
        .map_err(|error| AgentLoopError::tool(format!("Invalid x402 amount: {error}")))?;
    Ok(value / 1_000_000.0)
}

async fn create_x402_payment_signature(
    wallet: &LocalWallet,
    required: &X402PaymentRequired,
    selected: &X402PaymentRequirements,
) -> Result<String> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| AgentLoopError::tool(format!("System clock error: {error}")))?
        .as_secs();
    let valid_after = now.saturating_sub(600).to_string();
    let timeout = selected.max_timeout_seconds.max(1) as u64;
    let valid_before = (now + timeout.min(3600)).to_string();
    let nonce = random_bytes32_hex();
    let from = wallet.address_checksummed();

    let authorization = json!({
        "from": from,
        "to": selected.pay_to,
        "value": selected.amount,
        "validAfter": valid_after,
        "validBefore": valid_before,
        "nonce": nonce,
    });
    let signature = sign_eip3009_authorization(wallet, selected, &authorization)?;

    let mut payload = json!({
        "x402Version": 2,
        "payload": {
            "signature": signature,
            "authorization": authorization,
        },
        "accepted": selected,
    });
    if let Some(resource) = required.resource.as_ref() {
        payload["resource"] = serde_json::to_value(resource).map_err(|error| {
            AgentLoopError::tool(format!("Failed to encode x402 resource: {error}"))
        })?;
    }
    if let Some(extensions) = required.extensions.as_ref() {
        payload["extensions"] = extensions.clone();
    }
    let bytes = serde_json::to_vec(&payload)
        .map_err(|error| AgentLoopError::tool(format!("Failed to encode x402 payload: {error}")))?;
    Ok(BASE64_STANDARD.encode(bytes))
}

fn sign_eip3009_authorization(
    wallet: &LocalWallet,
    selected: &X402PaymentRequirements,
    authorization: &serde_json::Value,
) -> Result<String> {
    let chain_id = selected
        .network
        .strip_prefix("eip155:")
        .ok_or_else(|| AgentLoopError::tool("x402 network is not an EIP-155 network"))?
        .parse::<u64>()
        .map_err(|error| AgentLoopError::tool(format!("Invalid x402 chain id: {error}")))?;
    let name = selected
        .extra
        .get("name")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("USD Coin");
    let version = selected
        .extra
        .get("version")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("2");
    let digest = eip712::transfer_with_authorization_digest(
        &Domain {
            name,
            version,
            chain_id,
            verifying_contract: &selected.asset,
        },
        &TransferWithAuthorization {
            from: field(authorization, "from")?,
            to: field(authorization, "to")?,
            value: field(authorization, "value")?,
            valid_after: field(authorization, "validAfter")?,
            valid_before: field(authorization, "validBefore")?,
            nonce: field(authorization, "nonce")?,
        },
    )
    .map_err(|error| {
        AgentLoopError::tool(format!("Failed to build x402 EIP-712 payload: {error}"))
    })?;

    Ok(format!("0x{}", hex::encode(wallet.sign_prehash(&digest))))
}

/// Reads a required string field out of the authorization object built by
/// `create_x402_payment_signature`.
fn field<'a>(authorization: &'a serde_json::Value, name: &'static str) -> Result<&'a str> {
    authorization
        .get(name)
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| AgentLoopError::tool(format!("x402 authorization is missing {name}")))
}

fn random_bytes32_hex() -> String {
    let bytes: [u8; 32] = rand::random();
    format!("0x{}", hex::encode(bytes))
}

fn parse_x402_payment_response(headers: &reqwest::header::HeaderMap) -> serde_json::Value {
    headers
        .get("PAYMENT-RESPONSE")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| BASE64_STANDARD.decode(value).ok())
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or(serde_json::Value::Null)
}

fn request_hash(request: &MachinePaymentRequest) -> String {
    let mut hasher = Sha256::new();
    hasher.update(request.capability.as_bytes());
    hasher.update(request.operation.as_bytes());
    hasher.update(method_wire(&request.method).as_bytes());
    hasher.update(request.url.as_bytes());
    if let Some(body) = request.body.as_ref() {
        hasher.update(body.to_string().as_bytes());
    }
    hex::encode(hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Anvil development key #0. Public test vector, never a real key.
    const TEST_PRIVATE_KEY: &str =
        "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";

    fn test_wallet() -> LocalWallet {
        LocalWallet::from_private_key_hex(TEST_PRIVATE_KEY).expect("test wallet")
    }

    #[tokio::test]
    async fn x402_signature_header_contains_eip3009_authorization() {
        let wallet = test_wallet();
        let required = X402PaymentRequired {
            x402_version: 2,
            resource: Some(X402ResourceInfo {
                url: "https://parallelmpp.dev/api/search".to_string(),
                description: None,
                mime_type: None,
            }),
            accepts: vec![],
            extensions: None,
        };
        let selected = X402PaymentRequirements {
            scheme: "exact".to_string(),
            network: "eip155:8453".to_string(),
            asset: "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913".to_string(),
            amount: "10000".to_string(),
            pay_to: "0x0000000000000000000000000000000000000001".to_string(),
            max_timeout_seconds: 60,
            extra: json!({
                "assetTransferMethod": "eip3009",
                "name": "USD Coin",
                "version": "2",
            }),
        };

        let header = create_x402_payment_signature(&wallet, &required, &selected)
            .await
            .expect("signature header");
        let payload: serde_json::Value =
            serde_json::from_slice(&BASE64_STANDARD.decode(header).expect("base64"))
                .expect("payload json");

        assert_eq!(payload["x402Version"], 2);
        assert_eq!(payload["accepted"]["scheme"], "exact");
        assert_eq!(payload["accepted"]["network"], "eip155:8453");
        assert_eq!(
            payload["payload"]["authorization"]["from"],
            "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266"
        );
        assert_eq!(payload["payload"]["authorization"]["value"], "10000");
        assert!(
            payload["payload"]["signature"]
                .as_str()
                .is_some_and(|signature| signature.starts_with("0x") && signature.len() == 132)
        );
    }

    /// Fixed EIP-712 vector for `TransferWithAuthorization` on Base mainnet
    /// USDC, signed with Anvil key #0.
    ///
    /// Every input is pinned, so the signature is a constant. It is the
    /// equivalence check for the signing stack: a change that alters domain
    /// separation, struct hashing, or the v-byte encoding will move this value
    /// and break payments against every x402 facilitator.
    #[test]
    fn eip3009_signature_matches_known_vector() {
        let wallet = test_wallet();
        let selected = X402PaymentRequirements {
            scheme: "exact".to_string(),
            network: "eip155:8453".to_string(),
            asset: "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913".to_string(),
            amount: "10000".to_string(),
            pay_to: "0x0000000000000000000000000000000000000001".to_string(),
            max_timeout_seconds: 60,
            extra: json!({
                "assetTransferMethod": "eip3009",
                "name": "USD Coin",
                "version": "2",
            }),
        };
        let authorization = json!({
            "from": "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266",
            "to": "0x0000000000000000000000000000000000000001",
            "value": "10000",
            "validAfter": "1700000000",
            "validBefore": "1700003600",
            "nonce": "0x0000000000000000000000000000000000000000000000000000000000000001",
        });

        let signature =
            sign_eip3009_authorization(&wallet, &selected, &authorization).expect("signature");

        assert_eq!(
            signature,
            concat!(
                "0x6105f86ea8e1a854017dcc6189ddcb261a3d52653bb8663ba1aa476f24db3d5e",
                "6e6aea771913b82d1c2bc3bc3712913fae996768d063075d00cb26dea5742b2f1c",
            )
        );
    }

    /// Regression test for TM-AGENT-023: the payment HTTP client must not follow
    /// 30x redirects, which would otherwise bypass the policy host allowlist
    /// applied to the original request URL.
    #[tokio::test]
    async fn payment_http_client_does_not_follow_redirects() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/redirect"))
            .respond_with(
                ResponseTemplate::new(302).insert_header("location", "http://127.0.0.1:1/internal"),
            )
            .mount(&server)
            .await;

        let client = build_payment_http_client().expect("client");
        let response = client
            .get(format!("{}/redirect", server.uri()))
            .send()
            .await
            .expect("redirect response");

        assert_eq!(response.status().as_u16(), 302);
        assert_eq!(
            response
                .headers()
                .get("location")
                .and_then(|value| value.to_str().ok()),
            Some("http://127.0.0.1:1/internal"),
        );
    }
}
