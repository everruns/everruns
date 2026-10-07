// Machine-payment execution authority for server-backed tools.
//
// Decision: paid transport is internal infrastructure. Capabilities submit typed
// requests; this service resolves a wallet/policy, runs the selected native
// payment rail adapter, and records an auditable attempt.

use super::eip712::{self, Domain, LocalWallet, TransferWithAuthorization};
use crate::storage::StorageBackend;
use crate::storage::encryption::EncryptionService;
use crate::storage::models::{
    CreatePaymentAttemptRow, PaymentAccountRow, PaymentAttemptRow, PaymentPolicyRow,
};
use async_trait::async_trait;
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use everruns_contracts::error::{AgentLoopError, Result};
use everruns_contracts::typed_id::{AgentId, PaymentAttemptId, SessionId};
use everruns_core::payment::{
    MachinePaymentRequest, MachinePaymentResponse, PaymentMethod, PaymentRail,
};
use everruns_core::tool_execution::PaymentAuthority;
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
    /// The persisted input that caused this execution. Its server-recorded
    /// invocation is the only provenance that can prove an interactive,
    /// user-initiated turn (EVE-1187).
    input_message_id: Option<uuid::Uuid>,
    feature_flag_policy: crate::records::FeatureFlagPolicy,
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
            input_message_id: None,
            feature_flag_policy: crate::records::FeatureFlagPolicy::current(),
        }
    }

    /// Scope wallet resolution to the input that caused this execution.
    pub fn bound_to_input_message(mut self, input_message_id: uuid::Uuid) -> Self {
        self.input_message_id = Some(input_message_id);
        self
    }
}

struct SelectedPolicy {
    account: PaymentAccountRow,
    policy: PaymentPolicyRow,
    rail: PaymentRail,
}

#[async_trait]
impl PaymentAuthority for ServerPaymentAuthority {
    fn for_execution(&self, input_message_id: uuid::Uuid) -> Option<Arc<dyn PaymentAuthority>> {
        Some(Arc::new(
            self.clone().bound_to_input_message(input_message_id),
        ))
    }

    async fn execute_machine_payment(
        &self,
        session_id: SessionId,
        request: MachinePaymentRequest,
    ) -> Result<MachinePaymentResponse> {
        // Re-check durable policy at spend time: an already-loaded tool must not
        // bypass an organisation opt-out or a deployment kill switch.
        let flags = crate::services::org_feature_flags::resolve_org_feature_flags(
            &self.db,
            self.org_id,
            &self.feature_flag_policy,
        )
        .await
        .map_err(|error| {
            AgentLoopError::store(format!("Failed to resolve payment feature: {error}"))
        })?;
        if !flags.machine_payments {
            return Err(AgentLoopError::config(
                "feature_not_enabled: machine_payments",
            ));
        }
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
    /// The human user who interactively initiated the bound input, if any.
    ///
    /// THREAT[TM-AGENT-022]: an unattended turn (schedule, agent trigger, app
    /// channel, health check, eval) must not inherit the session owner's
    /// wallet. The only proof of an interactive, user-initiated turn is the
    /// management user `MessageService` records on the input's runtime
    /// invocation when an authenticated user sends it directly; every other
    /// ingress records none. The lookup is keyed by this session, so an input
    /// from another session proves nothing. No binding, no row, or no
    /// management user all fail closed: no user wallet is a candidate.
    /// Turns an interactive turn starts in other sessions through platform
    /// tools carry that same proven user, while an unattended turn cannot
    /// start them (platform tools require the same management user).
    async fn interactive_initiator(&self, session_id: SessionId) -> Result<Option<uuid::Uuid>> {
        let Some(input_message_id) = self.input_message_id else {
            return Ok(None);
        };
        self.db
            .runtime_invocation_management_user(session_id, input_message_id)
            .await
            .map_err(|error| {
                AgentLoopError::store(format!(
                    "Failed to resolve payment turn provenance: {error}"
                ))
            })
    }

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
        let agent_id = self.agent_id.or(session.agent_id);
        let agent_public_id = match agent_id {
            Some(agent_id) => self
                .db
                .get_agent_public_id(self.org_id, agent_id)
                .await
                .map_err(|error| {
                    AgentLoopError::store(format!("Failed to resolve payment agent: {error}"))
                })?,
            None => None,
        };
        let channel_public_id = match session.channel_id {
            Some(channel_id) => self
                .db
                .get_agent_channel_public_id(self.org_id, channel_id)
                .await
                .map_err(|error| {
                    AgentLoopError::store(format!("Failed to resolve payment endpoint: {error}"))
                })?,
            None => None,
        };
        let initiating_user_id = self.interactive_initiator(session_id).await?;
        let candidates = subject_candidates(
            session_id,
            agent_id,
            agent_public_id,
            session.virtual_user_id,
            initiating_user_id,
            channel_public_id,
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
            // session, agent, virtual user, interactive initiating user, or organization plus
            // capability, host, rail, and per-request limit before a payment is signed.
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

/// The `(subject_type, subject_id)` pairs a policy may be bound to for this
/// session. A policy authorizes a payment only on an exact match, so a subject
/// missing from here is a subject that can be stored and never applied.
///
/// `agent_public_id` and `channel_public_id` are resolved by the caller,
/// because both need a lookup. They are the identifiers the API and the UI
/// expose, and therefore the only ones an operator can put in a policy
/// (EVE-1130) — the typed ids taken off the session row spell internal uuids.
/// Those internal spellings are kept alongside rather than replaced: a policy
/// stored with one still applies, and dropping it would disable that policy
/// rather than fix anything.
///
/// There is deliberately no `app` candidate. The subject type is retired
/// (migration 152), and nothing produced a candidate for it even before that.
fn subject_candidates(
    session_id: SessionId,
    agent_id: Option<AgentId>,
    agent_public_id: Option<String>,
    virtual_user_id: Option<everruns_contracts::typed_id::VirtualUserId>,
    initiating_user_id: Option<uuid::Uuid>,
    channel_public_id: Option<String>,
) -> Vec<(&'static str, String)> {
    let mut candidates = vec![
        ("session", session_id.to_string()),
        ("session", session_id.uuid().to_string()),
        ("org", "org".to_string()),
    ];
    if let Some(channel_public_id) = channel_public_id {
        candidates.push(("agent_channel", channel_public_id));
    }
    if let Some(agent_public_id) = agent_public_id {
        candidates.push(("agent", agent_public_id));
    }
    if let Some(agent_id) = agent_id {
        candidates.push(("agent", agent_id.to_string()));
        candidates.push(("agent", agent_id.uuid().to_string()));
    }
    if let Some(virtual_user_id) = virtual_user_id {
        candidates.push(("virtual_user", virtual_user_id.to_string()));
        candidates.push(("virtual_user", virtual_user_id.uuid().to_string()));
    }
    // Only the proven interactive initiator (see `interactive_initiator`), never
    // the session's resolved owner: unattended work must not spend a human's
    // wallet (EVE-1187).
    if let Some(user_id) = initiating_user_id {
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
#[path = "authority_wallet_tests.rs"]
mod wallet_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn payment_execution_observes_org_revocation_before_request_or_spend() {
        use everruns_core::{DeploymentGrade, FeatureFlagGrade};
        let db = Arc::new(StorageBackend::test_database());
        let mut authority = ServerPaymentAuthority::new(db.clone(), None, 42, None);
        authority.feature_flag_policy =
            crate::records::FeatureFlagPolicy::from_env(DeploymentGrade::Prod)
                .with_grade("machine_payments", FeatureFlagGrade::Prod);
        // An invalid request cannot reach a transport, even when the flag is on.
        let request = MachinePaymentRequest {
            capability: "parallel".into(),
            operation: "search".into(),
            method: PaymentMethod::Post,
            url: String::new(),
            body: None,
            max_amount_usd: 0.01,
            rail_preference: vec![],
            metadata: json!({}),
        };
        let before = authority
            .execute_machine_payment(SessionId::new(), request.clone())
            .await
            .unwrap_err();
        assert!(!before.to_string().contains("feature_not_enabled"));
        db.replace_org_feature_flags(
            42,
            &std::collections::HashMap::from([("machine_payments".into(), false)]),
        )
        .await
        .unwrap();
        let after = authority
            .execute_machine_payment(SessionId::new(), request)
            .await
            .unwrap_err();
        assert!(after.to_string().contains("feature_not_enabled"));
    }

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

    /// EVE-1130: a policy only ever authorizes a payment if `subject_candidates`
    /// produces the exact `(subject_type, subject_id)` pair it was stored with.
    /// The `agent` subject is the one an operator reaches for, and the pair they
    /// can actually create is the agent's `public_id` — the identifier the API
    /// and the UI expose. Rendering the typed `AgentId` off the session row
    /// instead spells the internal uuid, which no stored policy can match, so
    /// the policy is accepted and then silently never applies.
    #[test]
    fn agent_candidates_include_the_public_id_an_operator_can_bind_to() {
        let session_id = SessionId::new();
        let agent_id = AgentId::new();
        let agent_public_id = AgentId::new().to_string();
        assert_ne!(agent_id.to_string(), agent_public_id);

        let candidates = subject_candidates(
            session_id,
            Some(agent_id),
            Some(agent_public_id.clone()),
            None,
            None,
            None,
        );

        assert!(
            candidates.contains(&("agent", agent_public_id.clone())),
            "the agent's public id must be a candidate: {candidates:?}"
        );
        // The internal spellings stay: a policy stored with one still applies,
        // and removing them would disable it rather than fix anything.
        assert!(candidates.contains(&("agent", agent_id.to_string())));
    }

    /// `agent_channel` is offered as a subject, so it has to resolve. A subject
    /// that can be selected and stored but never matched is worse than one that
    /// is absent: it reads as authority scoped to an endpoint while nothing
    /// enforces the scope.
    #[test]
    fn agent_channel_is_a_candidate_when_the_session_arrived_through_one() {
        let session_id = SessionId::new();
        let channel_public_id = "appchan_0199f0c2d4b17a3e9c1155aa77e30b41".to_string();

        let candidates = subject_candidates(
            session_id,
            None,
            None,
            None,
            None,
            Some(channel_public_id.clone()),
        );

        assert!(
            candidates.contains(&("agent_channel", channel_public_id)),
            "an endpoint-scoped policy must be reachable: {candidates:?}"
        );
    }

    /// `app` is retired. Nothing produced an `app` candidate even before this
    /// change, so every `app` policy the API accepted was already inert — which
    /// is why migration 152 converts them onto the agent instead of trusting
    /// that they were doing nothing.
    #[test]
    fn app_is_never_a_candidate() {
        let candidates = subject_candidates(
            SessionId::new(),
            Some(AgentId::new()),
            Some(AgentId::new().to_string()),
            None,
            None,
            Some("appchan_0199f0c2d4b17a3e9c1155aa77e30b41".to_string()),
        );
        assert!(
            !candidates.iter().any(|(kind, _)| *kind == "app"),
            "{candidates:?}"
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
