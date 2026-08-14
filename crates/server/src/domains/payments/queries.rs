use crate::domains::common::CommandError;
use crate::storage::models::{PaymentAccountRow, PaymentAttemptRow, PaymentPolicyRow};
use everruns_core::payment::PaymentRail;
use everruns_platform::payment::{
    PaymentAccount, PaymentAttempt, PaymentOwnerType, PaymentPolicy, PaymentStatus,
};
use everruns_provider::typed_id::{PaymentAccountId, PaymentAttemptId, PaymentPolicyId, SessionId};

pub fn parse_payment_account_id(input: &str) -> Result<uuid::Uuid, CommandError> {
    if let Ok(id) = PaymentAccountId::parse(input) {
        Ok(id.uuid())
    } else if let Ok(id) = uuid::Uuid::parse_str(input) {
        Ok(id)
    } else {
        Err(CommandError::bad_request(
            "Invalid payment account ID format",
        ))
    }
}

pub fn parse_payment_policy_id(input: &str) -> Result<uuid::Uuid, CommandError> {
    if let Ok(id) = PaymentPolicyId::parse(input) {
        Ok(id.uuid())
    } else if let Ok(id) = uuid::Uuid::parse_str(input) {
        Ok(id)
    } else {
        Err(CommandError::bad_request(
            "Invalid payment policy ID format",
        ))
    }
}

pub fn parse_session_id(input: &str) -> Result<uuid::Uuid, CommandError> {
    if let Ok(id) = SessionId::parse(input) {
        Ok(id.uuid())
    } else if let Ok(id) = uuid::Uuid::parse_str(input) {
        Ok(id)
    } else {
        Err(CommandError::bad_request("Invalid session ID format"))
    }
}

pub fn row_to_account(row: &PaymentAccountRow) -> PaymentAccount {
    PaymentAccount {
        id: PaymentAccountId::from_uuid(row.id),
        organization_id: everruns_core::org_public_id_from_internal(row.org_id),
        owner_type: PaymentOwnerType::from(row.owner_type.as_str()),
        owner_id: row.owner_id.clone(),
        rail: PaymentRail::from(row.rail.as_str()),
        label: row.label.clone(),
        public_address: row.public_address.clone(),
        status: PaymentStatus::from(row.status.as_str()),
        metadata: row.metadata.clone(),
        created_at: row.created_at,
        updated_at: row.updated_at,
    }
}

pub fn row_to_policy(row: &PaymentPolicyRow) -> PaymentPolicy {
    PaymentPolicy {
        id: PaymentPolicyId::from_uuid(row.id),
        organization_id: everruns_core::org_public_id_from_internal(row.org_id),
        payment_account_id: PaymentAccountId::from_uuid(row.payment_account_id),
        subject_type: row.subject_type.clone(),
        subject_id: row.subject_id.clone(),
        allowed_capabilities: row.allowed_capabilities.clone(),
        allowed_hosts: row.allowed_hosts.clone(),
        rail_preference: row
            .rail_preference
            .iter()
            .map(|rail| PaymentRail::from(rail.as_str()))
            .collect(),
        max_amount_usd_per_request: row.max_amount_usd_per_request,
        max_amount_usd_per_turn: row.max_amount_usd_per_turn,
        max_amount_usd_per_day: row.max_amount_usd_per_day,
        require_approval_above_usd: row.require_approval_above_usd,
        status: PaymentStatus::from(row.status.as_str()),
        metadata: row.metadata.clone(),
        created_at: row.created_at,
        updated_at: row.updated_at,
    }
}

pub fn row_to_attempt(row: &PaymentAttemptRow) -> PaymentAttempt {
    PaymentAttempt {
        id: PaymentAttemptId::from_uuid(row.id),
        organization_id: everruns_core::org_public_id_from_internal(row.org_id),
        payment_account_id: row.payment_account_id.map(PaymentAccountId::from_uuid),
        session_id: row
            .session_id
            .map(|id| SessionId::from_uuid(id).to_string()),
        capability: row.capability.clone(),
        operation: row.operation.clone(),
        rail: row.rail.as_deref().map(PaymentRail::from),
        amount_usd: row.amount_usd,
        currency: row.currency.clone(),
        target_url: row.target_url.clone(),
        request_hash: row.request_hash.clone(),
        status: PaymentStatus::from(row.status.as_str()),
        error_message: row.error_message.clone(),
        receipt: row.receipt.clone(),
        created_at: row.created_at,
        updated_at: row.updated_at,
    }
}
