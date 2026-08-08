use super::types::*;
use super::{PAYMENT_MANAGE, PAYMENT_VIEW, queries as q};
use crate::domains::common::*;
use crate::storage::models::{
    CreatePaymentAccountRow, CreatePaymentPolicyRow, UpdatePaymentAccountRow,
    UpdatePaymentPolicyRow,
};
use everruns_core::Policy;
use everruns_platform::payment::{PaymentAccount, PaymentAttempt, PaymentPolicy};
use serde::Deserialize;
use utoipa::ToSchema;

fn validate_owner_type(value: &str) -> Result<(), CommandError> {
    match value {
        "user" | "agent_identity" | "organization" => Ok(()),
        _ => Err(CommandError::bad_request("Invalid owner_type")),
    }
}

fn validate_rail(value: &str) -> Result<(), CommandError> {
    match value {
        "mpp_tempo" | "x402_base" => Ok(()),
        _ => Err(CommandError::bad_request("Invalid payment rail")),
    }
}

fn validate_status(value: &str) -> Result<(), CommandError> {
    match value {
        "active" | "disabled" => Ok(()),
        _ => Err(CommandError::bad_request("Invalid status")),
    }
}

fn validate_subject_type(value: &str) -> Result<(), CommandError> {
    match value {
        "user" | "agent_identity" | "agent" | "app" | "session" | "org" => Ok(()),
        _ => Err(CommandError::bad_request("Invalid subject_type")),
    }
}

fn validate_positive_limits(values: &[Option<f64>]) -> Result<(), CommandError> {
    if values.iter().flatten().any(|value| *value <= 0.0) {
        return Err(CommandError::bad_request("Payment limits must be positive"));
    }
    Ok(())
}

fn validate_required_signing_material(
    rail: &str,
    credential_encrypted: Option<&[u8]>,
) -> Result<(), CommandError> {
    if rail == "x402_base" && credential_encrypted.is_none() {
        return Err(CommandError::bad_request(
            "x402_base payment accounts require private_key",
        ));
    }
    Ok(())
}

fn encrypt_wallet_key(
    ctx: &Ctx,
    private_key: Option<String>,
) -> Result<Option<Vec<u8>>, CommandError> {
    let Some(private_key) = private_key else {
        return Ok(None);
    };
    if private_key.trim().is_empty() {
        return Err(CommandError::bad_request("private_key cannot be empty"));
    }
    let encryption = ctx
        .encryption
        .as_ref()
        .ok_or_else(|| CommandError::internal(anyhow::anyhow!("Encryption is not configured")))?;
    // THREAT[TM-CRYPTO-008]: Machine-payment wallet keys must not be stored in plaintext.
    // Mitigation: accept the key only at write time, encrypt it with the server envelope
    // encryption service, and never include it in payment account API responses.
    encryption
        .encrypt_string(private_key.trim())
        .map(Some)
        .map_err(|error| {
            CommandError::internal(anyhow::anyhow!("Failed to encrypt wallet key: {error}"))
        })
}

#[derive(Debug, Deserialize)]
pub struct CreatePaymentAccount(pub CreatePaymentAccountRequest);

impl CommandSchema for CreatePaymentAccount {
    fn param_schema() -> serde_json::Value {
        delegated_param_schema::<CreatePaymentAccountRequest>()
    }
}

impl Command for CreatePaymentAccount {
    type Output = PaymentAccount;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "create_payment_account",
            category: "payments",
            description: "Create a machine-payment wallet account.",
            method: "POST",
            path: "/v1/payments/accounts",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&PAYMENT_MANAGE)
    }

    async fn execute(self, ctx: &Ctx) -> Result<PaymentAccount, CommandError> {
        let req = self.0;
        validate_owner_type(&req.owner_type)?;
        validate_rail(&req.rail)?;
        let credential_encrypted = encrypt_wallet_key(ctx, req.private_key)?;
        validate_required_signing_material(&req.rail, credential_encrypted.as_deref())?;
        let row = ctx
            .db
            .create_payment_account(
                ctx.org_id(),
                CreatePaymentAccountRow {
                    owner_type: req.owner_type,
                    owner_id: req.owner_id,
                    rail: req.rail,
                    label: req.label,
                    public_address: req.public_address,
                    credential_encrypted,
                    metadata: req.metadata.unwrap_or_else(|| serde_json::json!({})),
                },
            )
            .await
            .map_err(classify_anyhow)?;
        Ok(q::row_to_account(&row))
    }
}

inventory::submit! { CommandDescriptor::of::<CreatePaymentAccount>() }

#[derive(Debug, Default, Deserialize, ToSchema)]
pub struct ListPaymentAccounts {
    pub owner_type: Option<String>,
    pub owner_id: Option<String>,
}

impl Command for ListPaymentAccounts {
    type Output = Vec<PaymentAccount>;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "list_payment_accounts",
            category: "payments",
            description: "List machine-payment wallet accounts.",
            method: "GET",
            path: "/v1/payments/accounts",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&PAYMENT_VIEW)
    }

    async fn execute(self, ctx: &Ctx) -> Result<Vec<PaymentAccount>, CommandError> {
        let rows = ctx
            .db
            .list_payment_accounts(
                ctx.org_id(),
                self.owner_type.as_deref(),
                self.owner_id.as_deref(),
            )
            .await
            .map_err(classify_anyhow)?;
        Ok(rows.iter().map(q::row_to_account).collect())
    }
}

inventory::submit! { CommandDescriptor::of::<ListPaymentAccounts>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct GetPaymentAccount {
    pub payment_account_id: String,
}

impl Command for GetPaymentAccount {
    type Output = PaymentAccount;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "get_payment_account",
            category: "payments",
            description: "Get a machine-payment wallet account.",
            method: "GET",
            path: "/v1/payments/accounts/{payment_account_id}",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&PAYMENT_VIEW)
    }

    async fn execute(self, ctx: &Ctx) -> Result<PaymentAccount, CommandError> {
        let id = q::parse_payment_account_id(&self.payment_account_id)?;
        let row = ctx
            .db
            .get_payment_account(ctx.org_id(), id)
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Payment account"))?;
        Ok(q::row_to_account(&row))
    }
}

inventory::submit! { CommandDescriptor::of::<GetPaymentAccount>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdatePaymentAccountCmd {
    pub payment_account_id: String,
    pub label: Option<String>,
    pub public_address: Option<Option<String>>,
    pub private_key: Option<String>,
    /// Current lifecycle status.
    pub status: Option<String>,
    /// Free-form metadata attached to this resource.
    pub metadata: Option<serde_json::Value>,
}

impl Command for UpdatePaymentAccountCmd {
    type Output = PaymentAccount;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "update_payment_account",
            category: "payments",
            description: "Update a machine-payment wallet account.",
            method: "PATCH",
            path: "/v1/payments/accounts/{payment_account_id}",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&PAYMENT_MANAGE)
    }

    async fn execute(self, ctx: &Ctx) -> Result<PaymentAccount, CommandError> {
        if let Some(status) = self.status.as_deref() {
            validate_status(status)?;
        }
        let credential_encrypted = encrypt_wallet_key(ctx, self.private_key)?;
        let id = q::parse_payment_account_id(&self.payment_account_id)?;
        let row = ctx
            .db
            .update_payment_account(
                ctx.org_id(),
                id,
                UpdatePaymentAccountRow {
                    label: self.label,
                    public_address: self.public_address,
                    credential_encrypted,
                    status: self.status,
                    metadata: self.metadata,
                },
            )
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Payment account"))?;
        validate_required_signing_material(&row.rail, row.credential_encrypted.as_deref())?;
        Ok(q::row_to_account(&row))
    }
}

inventory::submit! { CommandDescriptor::of::<UpdatePaymentAccountCmd>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct DisablePaymentAccount {
    pub payment_account_id: String,
}

impl Command for DisablePaymentAccount {
    type Output = PaymentDeleteResult;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "disable_payment_account",
            category: "payments",
            description: "Disable a machine-payment wallet account.",
            method: "DELETE",
            path: "/v1/payments/accounts/{payment_account_id}",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&PAYMENT_MANAGE)
    }

    async fn execute(self, ctx: &Ctx) -> Result<PaymentDeleteResult, CommandError> {
        let id = q::parse_payment_account_id(&self.payment_account_id)?;
        let row = ctx
            .db
            .update_payment_account(
                ctx.org_id(),
                id,
                UpdatePaymentAccountRow {
                    status: Some("disabled".to_string()),
                    ..Default::default()
                },
            )
            .await
            .map_err(classify_anyhow)?;
        Ok(PaymentDeleteResult {
            disabled: row.is_some(),
        })
    }
}

inventory::submit! { CommandDescriptor::of::<DisablePaymentAccount>() }

#[derive(Debug, Deserialize)]
pub struct CreatePaymentPolicy(pub CreatePaymentPolicyRequest);

impl CommandSchema for CreatePaymentPolicy {
    fn param_schema() -> serde_json::Value {
        delegated_param_schema::<CreatePaymentPolicyRequest>()
    }
}

impl Command for CreatePaymentPolicy {
    type Output = PaymentPolicy;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "create_payment_policy",
            category: "payments",
            description: "Create a machine-payment spend policy.",
            method: "POST",
            path: "/v1/payments/policies",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&PAYMENT_MANAGE)
    }

    async fn execute(self, ctx: &Ctx) -> Result<PaymentPolicy, CommandError> {
        let req = self.0;
        validate_subject_type(&req.subject_type)?;
        for rail in &req.rail_preference {
            validate_rail(rail)?;
        }
        validate_positive_limits(&[
            req.max_amount_usd_per_request,
            req.max_amount_usd_per_turn,
            req.max_amount_usd_per_day,
            req.require_approval_above_usd,
        ])?;
        let account_id = q::parse_payment_account_id(&req.payment_account_id)?;
        let _account = ctx
            .db
            .get_payment_account(ctx.org_id(), account_id)
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Payment account"))?;
        let row = ctx
            .db
            .create_payment_policy(
                ctx.org_id(),
                CreatePaymentPolicyRow {
                    payment_account_id: account_id,
                    subject_type: req.subject_type,
                    subject_id: req.subject_id,
                    allowed_capabilities: req.allowed_capabilities,
                    allowed_hosts: req.allowed_hosts,
                    rail_preference: req.rail_preference,
                    max_amount_usd_per_request: req.max_amount_usd_per_request,
                    max_amount_usd_per_turn: req.max_amount_usd_per_turn,
                    max_amount_usd_per_day: req.max_amount_usd_per_day,
                    require_approval_above_usd: req.require_approval_above_usd,
                    metadata: req.metadata.unwrap_or_else(|| serde_json::json!({})),
                },
            )
            .await
            .map_err(classify_anyhow)?;
        Ok(q::row_to_policy(&row))
    }
}

inventory::submit! { CommandDescriptor::of::<CreatePaymentPolicy>() }

#[derive(Debug, Default, Deserialize, ToSchema)]
pub struct ListPaymentPolicies {
    pub payment_account_id: Option<String>,
    pub subject_type: Option<String>,
    pub subject_id: Option<String>,
}

impl Command for ListPaymentPolicies {
    type Output = Vec<PaymentPolicy>;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "list_payment_policies",
            category: "payments",
            description: "List machine-payment spend policies.",
            method: "GET",
            path: "/v1/payments/policies",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&PAYMENT_VIEW)
    }

    async fn execute(self, ctx: &Ctx) -> Result<Vec<PaymentPolicy>, CommandError> {
        let account_id = self
            .payment_account_id
            .as_deref()
            .map(q::parse_payment_account_id)
            .transpose()?;
        let rows = ctx
            .db
            .list_payment_policies(
                ctx.org_id(),
                account_id,
                self.subject_type.as_deref(),
                self.subject_id.as_deref(),
            )
            .await
            .map_err(classify_anyhow)?;
        Ok(rows.iter().map(q::row_to_policy).collect())
    }
}

inventory::submit! { CommandDescriptor::of::<ListPaymentPolicies>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct GetPaymentPolicy {
    pub payment_policy_id: String,
}

impl Command for GetPaymentPolicy {
    type Output = PaymentPolicy;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "get_payment_policy",
            category: "payments",
            description: "Get a machine-payment spend policy.",
            method: "GET",
            path: "/v1/payments/policies/{payment_policy_id}",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&PAYMENT_VIEW)
    }

    async fn execute(self, ctx: &Ctx) -> Result<PaymentPolicy, CommandError> {
        let id = q::parse_payment_policy_id(&self.payment_policy_id)?;
        let row = ctx
            .db
            .get_payment_policy(ctx.org_id(), id)
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Payment policy"))?;
        Ok(q::row_to_policy(&row))
    }
}

inventory::submit! { CommandDescriptor::of::<GetPaymentPolicy>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdatePaymentPolicyCmd {
    pub payment_policy_id: String,
    pub allowed_capabilities: Option<Vec<String>>,
    pub allowed_hosts: Option<Vec<String>>,
    pub rail_preference: Option<Vec<String>>,
    pub max_amount_usd_per_request: Option<Option<f64>>,
    pub max_amount_usd_per_turn: Option<Option<f64>>,
    pub max_amount_usd_per_day: Option<Option<f64>>,
    pub require_approval_above_usd: Option<Option<f64>>,
    /// Current lifecycle status.
    pub status: Option<String>,
    /// Free-form metadata attached to this resource.
    pub metadata: Option<serde_json::Value>,
}

impl Command for UpdatePaymentPolicyCmd {
    type Output = PaymentPolicy;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "update_payment_policy",
            category: "payments",
            description: "Update a machine-payment spend policy.",
            method: "PATCH",
            path: "/v1/payments/policies/{payment_policy_id}",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&PAYMENT_MANAGE)
    }

    async fn execute(self, ctx: &Ctx) -> Result<PaymentPolicy, CommandError> {
        if let Some(status) = self.status.as_deref() {
            validate_status(status)?;
        }
        if let Some(rails) = self.rail_preference.as_ref() {
            for rail in rails {
                validate_rail(rail)?;
            }
        }
        validate_positive_limits(&[
            self.max_amount_usd_per_request.flatten(),
            self.max_amount_usd_per_turn.flatten(),
            self.max_amount_usd_per_day.flatten(),
            self.require_approval_above_usd.flatten(),
        ])?;
        let id = q::parse_payment_policy_id(&self.payment_policy_id)?;
        let row = ctx
            .db
            .update_payment_policy(
                ctx.org_id(),
                id,
                UpdatePaymentPolicyRow {
                    allowed_capabilities: self.allowed_capabilities,
                    allowed_hosts: self.allowed_hosts,
                    rail_preference: self.rail_preference,
                    max_amount_usd_per_request: self.max_amount_usd_per_request,
                    max_amount_usd_per_turn: self.max_amount_usd_per_turn,
                    max_amount_usd_per_day: self.max_amount_usd_per_day,
                    require_approval_above_usd: self.require_approval_above_usd,
                    status: self.status,
                    metadata: self.metadata,
                },
            )
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Payment policy"))?;
        Ok(q::row_to_policy(&row))
    }
}

inventory::submit! { CommandDescriptor::of::<UpdatePaymentPolicyCmd>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct DisablePaymentPolicy {
    pub payment_policy_id: String,
}

impl Command for DisablePaymentPolicy {
    type Output = PaymentDeleteResult;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "disable_payment_policy",
            category: "payments",
            description: "Disable a machine-payment spend policy.",
            method: "DELETE",
            path: "/v1/payments/policies/{payment_policy_id}",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&PAYMENT_MANAGE)
    }

    async fn execute(self, ctx: &Ctx) -> Result<PaymentDeleteResult, CommandError> {
        let id = q::parse_payment_policy_id(&self.payment_policy_id)?;
        let row = ctx
            .db
            .update_payment_policy(
                ctx.org_id(),
                id,
                UpdatePaymentPolicyRow {
                    status: Some("disabled".to_string()),
                    ..Default::default()
                },
            )
            .await
            .map_err(classify_anyhow)?;
        Ok(PaymentDeleteResult {
            disabled: row.is_some(),
        })
    }
}

inventory::submit! { CommandDescriptor::of::<DisablePaymentPolicy>() }

#[derive(Debug, Default, Deserialize, ToSchema)]
pub struct ListPaymentAttempts {
    /// Session's prefixed public identifier.
    pub session_id: Option<String>,
    /// Maximum number of items returned in this page.
    pub limit: i64,
}

impl Command for ListPaymentAttempts {
    type Output = Vec<PaymentAttempt>;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "list_payment_attempts",
            category: "payments",
            description: "List machine-payment attempts.",
            method: "GET",
            path: "/v1/payments/attempts",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&PAYMENT_VIEW)
    }

    async fn execute(self, ctx: &Ctx) -> Result<Vec<PaymentAttempt>, CommandError> {
        let session_id = self
            .session_id
            .as_deref()
            .map(q::parse_session_id)
            .transpose()?;
        let rows = ctx
            .db
            .list_payment_attempts(ctx.org_id(), session_id, self.limit.clamp(1, 200))
            .await
            .map_err(classify_anyhow)?;
        Ok(rows.iter().map(q::row_to_attempt).collect())
    }
}

inventory::submit! { CommandDescriptor::of::<ListPaymentAttempts>() }

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::{EncryptionService, StorageBackend};
    use everruns_core::{Caller, DEFAULT_ORG_ID};
    use std::sync::Arc;
    use uuid::Uuid;

    fn test_ctx() -> Ctx {
        let encryption =
            EncryptionService::new("kek-v1:8B3uCQ4Znx45hl5nB+PKVriRrj/KtEVM+wBZ2VGa9vY=", &[])
                .unwrap();
        Ctx::minimal_for_test(
            Caller::internal(DEFAULT_ORG_ID),
            Arc::new(StorageBackend::in_memory()),
            Some(Arc::new(encryption)),
        )
    }

    fn x402_account_request(private_key: Option<String>) -> CreatePaymentAccountRequest {
        CreatePaymentAccountRequest {
            owner_type: "organization".to_string(),
            owner_id: "org".to_string(),
            rail: "x402_base".to_string(),
            label: "Base USDC".to_string(),
            public_address: Some("0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266".to_string()),
            private_key,
            metadata: None,
        }
    }

    #[tokio::test]
    async fn create_x402_account_requires_private_key() {
        let err = CreatePaymentAccount(x402_account_request(None))
            .execute(&test_ctx())
            .await
            .expect_err("x402 accounts must be backed by signing material");

        assert!(
            matches!(err, CommandError { kind: CommandErrorKind::BadRequest(message), .. } if message.contains("private_key"))
        );
    }

    #[tokio::test]
    async fn account_get_accepts_typed_and_uuid_ids() {
        let ctx = test_ctx();
        let account = CreatePaymentAccount(x402_account_request(Some(
            "0x59c6995e998f97a5a0044966f094538447b6f9b16e5c4f0f4f2b331e1a1a7d3f".to_string(),
        )))
        .execute(&ctx)
        .await
        .unwrap();

        let by_typed = GetPaymentAccount {
            payment_account_id: account.id.to_string(),
        }
        .execute(&ctx)
        .await
        .unwrap();
        let by_uuid = GetPaymentAccount {
            payment_account_id: account.id.uuid().to_string(),
        }
        .execute(&ctx)
        .await
        .unwrap();

        assert_eq!(by_typed.id, account.id);
        assert_eq!(by_uuid.id, account.id);
    }

    #[tokio::test]
    async fn policy_get_accepts_typed_and_uuid_ids() {
        let ctx = test_ctx();
        let account = CreatePaymentAccount(x402_account_request(Some(
            "0x59c6995e998f97a5a0044966f094538447b6f9b16e5c4f0f4f2b331e1a1a7d3f".to_string(),
        )))
        .execute(&ctx)
        .await
        .unwrap();
        let policy = CreatePaymentPolicy(CreatePaymentPolicyRequest {
            payment_account_id: account.id.to_string(),
            subject_type: "org".to_string(),
            subject_id: "org".to_string(),
            allowed_capabilities: vec!["parallel".to_string()],
            allowed_hosts: vec!["parallelmpp.dev".to_string()],
            rail_preference: vec!["x402_base".to_string()],
            max_amount_usd_per_request: Some(0.01),
            max_amount_usd_per_turn: None,
            max_amount_usd_per_day: None,
            require_approval_above_usd: None,
            metadata: None,
        })
        .execute(&ctx)
        .await
        .unwrap();

        let by_typed = GetPaymentPolicy {
            payment_policy_id: policy.id.to_string(),
        }
        .execute(&ctx)
        .await
        .unwrap();
        let by_uuid = GetPaymentPolicy {
            payment_policy_id: policy.id.uuid().to_string(),
        }
        .execute(&ctx)
        .await
        .unwrap();

        assert_eq!(by_typed.id, policy.id);
        assert_eq!(by_uuid.id, policy.id);
    }

    #[tokio::test]
    async fn disable_returns_false_for_missing_rows() {
        let ctx = test_ctx();

        let account = DisablePaymentAccount {
            payment_account_id: Uuid::now_v7().to_string(),
        }
        .execute(&ctx)
        .await
        .unwrap();
        let policy = DisablePaymentPolicy {
            payment_policy_id: Uuid::now_v7().to_string(),
        }
        .execute(&ctx)
        .await
        .unwrap();

        assert!(!account.disabled);
        assert!(!policy.disabled);
    }
}
