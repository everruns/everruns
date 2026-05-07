// In-memory storage: machine payments.

use super::super::models::*;
use super::InMemoryDatabase;
use anyhow::Result;
use uuid::Uuid;

impl InMemoryDatabase {
    pub async fn create_payment_account(
        &self,
        org_id: i64,
        input: CreatePaymentAccountRow,
    ) -> Result<PaymentAccountRow> {
        let now = Self::now();
        let id = Uuid::now_v7();
        let row = PaymentAccountRow {
            id,
            org_id,
            owner_type: input.owner_type,
            owner_id: input.owner_id,
            rail: input.rail,
            label: input.label,
            public_address: input.public_address,
            credential_encrypted: input.credential_encrypted,
            status: "active".to_string(),
            metadata: input.metadata,
            created_at: now,
            updated_at: now,
        };
        self.payment_accounts.write().insert(id, row.clone());
        Ok(row)
    }

    pub async fn list_payment_accounts(
        &self,
        org_id: i64,
        owner_type: Option<&str>,
        owner_id: Option<&str>,
    ) -> Result<Vec<PaymentAccountRow>> {
        let mut rows: Vec<_> = self
            .payment_accounts
            .read()
            .values()
            .filter(|row| {
                row.org_id == org_id
                    && owner_type.is_none_or(|value| row.owner_type == value)
                    && owner_id.is_none_or(|value| row.owner_id == value)
            })
            .cloned()
            .collect();
        rows.sort_by_key(|row| std::cmp::Reverse(row.created_at));
        Ok(rows)
    }

    pub async fn get_payment_account(
        &self,
        org_id: i64,
        id: Uuid,
    ) -> Result<Option<PaymentAccountRow>> {
        Ok(self
            .payment_accounts
            .read()
            .get(&id)
            .filter(|row| row.org_id == org_id)
            .cloned())
    }

    pub async fn update_payment_account(
        &self,
        org_id: i64,
        id: Uuid,
        input: UpdatePaymentAccountRow,
    ) -> Result<Option<PaymentAccountRow>> {
        let mut accounts = self.payment_accounts.write();
        let Some(row) = accounts.get_mut(&id) else {
            return Ok(None);
        };
        if row.org_id != org_id {
            return Ok(None);
        }
        if let Some(label) = input.label {
            row.label = label;
        }
        if let Some(public_address) = input.public_address {
            row.public_address = public_address;
        }
        if let Some(credential_encrypted) = input.credential_encrypted {
            row.credential_encrypted = Some(credential_encrypted);
        }
        if let Some(status) = input.status {
            row.status = status;
        }
        if let Some(metadata) = input.metadata {
            row.metadata = metadata;
        }
        row.updated_at = Self::now();
        Ok(Some(row.clone()))
    }

    pub async fn create_payment_policy(
        &self,
        org_id: i64,
        input: CreatePaymentPolicyRow,
    ) -> Result<PaymentPolicyRow> {
        let now = Self::now();
        let id = Uuid::now_v7();
        let row = PaymentPolicyRow {
            id,
            org_id,
            payment_account_id: input.payment_account_id,
            subject_type: input.subject_type,
            subject_id: input.subject_id,
            allowed_capabilities: input.allowed_capabilities,
            allowed_hosts: input.allowed_hosts,
            rail_preference: input.rail_preference,
            max_amount_usd_per_request: input.max_amount_usd_per_request,
            max_amount_usd_per_turn: input.max_amount_usd_per_turn,
            max_amount_usd_per_day: input.max_amount_usd_per_day,
            require_approval_above_usd: input.require_approval_above_usd,
            status: "active".to_string(),
            metadata: input.metadata,
            created_at: now,
            updated_at: now,
        };
        self.payment_policies.write().insert(id, row.clone());
        Ok(row)
    }

    pub async fn list_payment_policies(
        &self,
        org_id: i64,
        payment_account_id: Option<Uuid>,
        subject_type: Option<&str>,
        subject_id: Option<&str>,
    ) -> Result<Vec<PaymentPolicyRow>> {
        let mut rows: Vec<_> = self
            .payment_policies
            .read()
            .values()
            .filter(|row| {
                row.org_id == org_id
                    && payment_account_id.is_none_or(|id| row.payment_account_id == id)
                    && subject_type.is_none_or(|value| row.subject_type == value)
                    && subject_id.is_none_or(|value| row.subject_id == value)
            })
            .cloned()
            .collect();
        rows.sort_by_key(|row| std::cmp::Reverse(row.created_at));
        Ok(rows)
    }

    pub async fn get_payment_policy(
        &self,
        org_id: i64,
        id: Uuid,
    ) -> Result<Option<PaymentPolicyRow>> {
        Ok(self
            .payment_policies
            .read()
            .get(&id)
            .filter(|row| row.org_id == org_id)
            .cloned())
    }

    pub async fn update_payment_policy(
        &self,
        org_id: i64,
        id: Uuid,
        input: UpdatePaymentPolicyRow,
    ) -> Result<Option<PaymentPolicyRow>> {
        let mut policies = self.payment_policies.write();
        let Some(row) = policies.get_mut(&id) else {
            return Ok(None);
        };
        if row.org_id != org_id {
            return Ok(None);
        }
        if let Some(value) = input.allowed_capabilities {
            row.allowed_capabilities = value;
        }
        if let Some(value) = input.allowed_hosts {
            row.allowed_hosts = value;
        }
        if let Some(value) = input.rail_preference {
            row.rail_preference = value;
        }
        if let Some(value) = input.max_amount_usd_per_request {
            row.max_amount_usd_per_request = value;
        }
        if let Some(value) = input.max_amount_usd_per_turn {
            row.max_amount_usd_per_turn = value;
        }
        if let Some(value) = input.max_amount_usd_per_day {
            row.max_amount_usd_per_day = value;
        }
        if let Some(value) = input.require_approval_above_usd {
            row.require_approval_above_usd = value;
        }
        if let Some(status) = input.status {
            row.status = status;
        }
        if let Some(metadata) = input.metadata {
            row.metadata = metadata;
        }
        row.updated_at = Self::now();
        Ok(Some(row.clone()))
    }

    pub async fn list_payment_attempts(
        &self,
        org_id: i64,
        session_id: Option<Uuid>,
        limit: i64,
    ) -> Result<Vec<PaymentAttemptRow>> {
        let mut rows: Vec<_> = self
            .payment_attempts
            .read()
            .values()
            .filter(|row| {
                row.org_id == org_id && session_id.is_none_or(|id| row.session_id == Some(id))
            })
            .cloned()
            .collect();
        rows.sort_by_key(|row| std::cmp::Reverse(row.created_at));
        rows.truncate(limit.max(0) as usize);
        Ok(rows)
    }

    pub async fn create_payment_attempt(
        &self,
        org_id: i64,
        input: CreatePaymentAttemptRow,
    ) -> Result<PaymentAttemptRow> {
        let now = Self::now();
        let id = Uuid::now_v7();
        let row = PaymentAttemptRow {
            id,
            org_id,
            payment_account_id: input.payment_account_id,
            session_id: input.session_id,
            capability: input.capability,
            operation: input.operation,
            rail: input.rail,
            amount_usd: input.amount_usd,
            currency: input.currency,
            target_url: input.target_url,
            request_hash: input.request_hash,
            status: input.status,
            error_message: input.error_message,
            receipt: input.receipt,
            created_at: now,
            updated_at: now,
        };
        self.payment_attempts.write().insert(id, row.clone());
        Ok(row)
    }
}
