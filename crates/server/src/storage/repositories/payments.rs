// PostgreSQL repository: machine payment accounts, policies, and attempts.

use super::super::models::*;
use super::Database;
use anyhow::Result;
use uuid::Uuid;

const ACCOUNT_COLUMNS: &str = "id, org_id, owner_type, owner_id, rail, label, public_address, credential_encrypted, status, metadata, created_at, updated_at";
const POLICY_COLUMNS: &str = "id, org_id, payment_account_id, subject_type, subject_id, allowed_capabilities, allowed_hosts, rail_preference, max_amount_usd_per_request, max_amount_usd_per_turn, max_amount_usd_per_day, require_approval_above_usd, status, metadata, created_at, updated_at";
const ATTEMPT_COLUMNS: &str = "id, org_id, payment_account_id, session_id, capability, operation, rail, amount_usd, currency, target_url, request_hash, status, error_message, receipt, created_at, updated_at";

impl Database {
    pub async fn create_payment_account(
        &self,
        org_id: i64,
        input: CreatePaymentAccountRow,
    ) -> Result<PaymentAccountRow> {
        let row = sqlx::query_as::<_, PaymentAccountRow>(
            r#"
            INSERT INTO payment_accounts
                (org_id, owner_type, owner_id, rail, label, public_address, credential_encrypted, metadata)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
            RETURNING id, org_id, owner_type, owner_id, rail, label, public_address, credential_encrypted,
                      status, metadata, created_at, updated_at
            "#,
        )
        .bind(org_id)
        .bind(input.owner_type)
        .bind(input.owner_id)
        .bind(input.rail)
        .bind(input.label)
        .bind(input.public_address)
        .bind(input.credential_encrypted)
        .bind(input.metadata)
        .fetch_one(&self.pool)
        .await?;
        Ok(row)
    }

    pub async fn list_payment_accounts(
        &self,
        org_id: i64,
        owner_type: Option<&str>,
        owner_id: Option<&str>,
    ) -> Result<Vec<PaymentAccountRow>> {
        let sql = format!(
            "SELECT {ACCOUNT_COLUMNS} FROM payment_accounts \
             WHERE org_id = $1 AND ($2::text IS NULL OR owner_type = $2) \
               AND ($3::text IS NULL OR owner_id = $3) \
             ORDER BY created_at DESC"
        );
        Ok(
            sqlx::query_as::<_, PaymentAccountRow>(sqlx::AssertSqlSafe(sql.as_str()))
                .bind(org_id)
                .bind(owner_type)
                .bind(owner_id)
                .fetch_all(&self.pool)
                .await?,
        )
    }

    pub async fn get_payment_account(
        &self,
        org_id: i64,
        id: Uuid,
    ) -> Result<Option<PaymentAccountRow>> {
        let sql =
            format!("SELECT {ACCOUNT_COLUMNS} FROM payment_accounts WHERE org_id = $1 AND id = $2");
        Ok(
            sqlx::query_as::<_, PaymentAccountRow>(sqlx::AssertSqlSafe(sql.as_str()))
                .bind(org_id)
                .bind(id)
                .fetch_optional(&self.pool)
                .await?,
        )
    }

    pub async fn update_payment_account(
        &self,
        org_id: i64,
        id: Uuid,
        input: UpdatePaymentAccountRow,
    ) -> Result<Option<PaymentAccountRow>> {
        let row = sqlx::query_as::<_, PaymentAccountRow>(
            r#"
            UPDATE payment_accounts
            SET
                label = COALESCE($3, label),
                public_address = CASE WHEN $4 THEN $5 ELSE public_address END,
                credential_encrypted = COALESCE($6, credential_encrypted),
                status = COALESCE($7, status),
                metadata = COALESCE($8, metadata),
                updated_at = NOW()
            WHERE org_id = $1 AND id = $2
            RETURNING id, org_id, owner_type, owner_id, rail, label, public_address, credential_encrypted,
                      status, metadata, created_at, updated_at
            "#,
        )
        .bind(org_id)
        .bind(id)
        .bind(input.label)
        .bind(input.public_address.is_some())
        .bind(input.public_address.flatten())
        .bind(input.credential_encrypted)
        .bind(input.status)
        .bind(input.metadata)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    pub async fn create_payment_policy(
        &self,
        org_id: i64,
        input: CreatePaymentPolicyRow,
    ) -> Result<PaymentPolicyRow> {
        let row = sqlx::query_as::<_, PaymentPolicyRow>(
            r#"
            INSERT INTO payment_policies (
                org_id, payment_account_id, subject_type, subject_id,
                allowed_capabilities, allowed_hosts, rail_preference,
                max_amount_usd_per_request, max_amount_usd_per_turn,
                max_amount_usd_per_day, require_approval_above_usd, metadata
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
            RETURNING id, org_id, payment_account_id, subject_type, subject_id,
                      allowed_capabilities, allowed_hosts, rail_preference,
                      max_amount_usd_per_request, max_amount_usd_per_turn,
                      max_amount_usd_per_day, require_approval_above_usd,
                      status, metadata, created_at, updated_at
            "#,
        )
        .bind(org_id)
        .bind(input.payment_account_id)
        .bind(input.subject_type)
        .bind(input.subject_id)
        .bind(input.allowed_capabilities)
        .bind(input.allowed_hosts)
        .bind(input.rail_preference)
        .bind(input.max_amount_usd_per_request)
        .bind(input.max_amount_usd_per_turn)
        .bind(input.max_amount_usd_per_day)
        .bind(input.require_approval_above_usd)
        .bind(input.metadata)
        .fetch_one(&self.pool)
        .await?;
        Ok(row)
    }

    pub async fn list_payment_policies(
        &self,
        org_id: i64,
        payment_account_id: Option<Uuid>,
        subject_type: Option<&str>,
        subject_id: Option<&str>,
    ) -> Result<Vec<PaymentPolicyRow>> {
        let sql = format!(
            "SELECT {POLICY_COLUMNS} FROM payment_policies \
             WHERE org_id = $1 AND ($2::uuid IS NULL OR payment_account_id = $2) \
               AND ($3::text IS NULL OR subject_type = $3) \
               AND ($4::text IS NULL OR subject_id = $4) \
             ORDER BY created_at DESC"
        );
        Ok(
            sqlx::query_as::<_, PaymentPolicyRow>(sqlx::AssertSqlSafe(sql.as_str()))
                .bind(org_id)
                .bind(payment_account_id)
                .bind(subject_type)
                .bind(subject_id)
                .fetch_all(&self.pool)
                .await?,
        )
    }

    pub async fn get_payment_policy(
        &self,
        org_id: i64,
        id: Uuid,
    ) -> Result<Option<PaymentPolicyRow>> {
        let sql =
            format!("SELECT {POLICY_COLUMNS} FROM payment_policies WHERE org_id = $1 AND id = $2");
        Ok(
            sqlx::query_as::<_, PaymentPolicyRow>(sqlx::AssertSqlSafe(sql.as_str()))
                .bind(org_id)
                .bind(id)
                .fetch_optional(&self.pool)
                .await?,
        )
    }

    pub async fn update_payment_policy(
        &self,
        org_id: i64,
        id: Uuid,
        input: UpdatePaymentPolicyRow,
    ) -> Result<Option<PaymentPolicyRow>> {
        let row = sqlx::query_as::<_, PaymentPolicyRow>(
            r#"
            UPDATE payment_policies
            SET
                allowed_capabilities = COALESCE($3, allowed_capabilities),
                allowed_hosts = COALESCE($4, allowed_hosts),
                rail_preference = COALESCE($5, rail_preference),
                max_amount_usd_per_request = CASE WHEN $6 THEN $7 ELSE max_amount_usd_per_request END,
                max_amount_usd_per_turn = CASE WHEN $8 THEN $9 ELSE max_amount_usd_per_turn END,
                max_amount_usd_per_day = CASE WHEN $10 THEN $11 ELSE max_amount_usd_per_day END,
                require_approval_above_usd = CASE WHEN $12 THEN $13 ELSE require_approval_above_usd END,
                status = COALESCE($14, status),
                metadata = COALESCE($15, metadata),
                updated_at = NOW()
            WHERE org_id = $1 AND id = $2
            RETURNING id, org_id, payment_account_id, subject_type, subject_id,
                      allowed_capabilities, allowed_hosts, rail_preference,
                      max_amount_usd_per_request, max_amount_usd_per_turn,
                      max_amount_usd_per_day, require_approval_above_usd,
                      status, metadata, created_at, updated_at
            "#,
        )
        .bind(org_id)
        .bind(id)
        .bind(input.allowed_capabilities)
        .bind(input.allowed_hosts)
        .bind(input.rail_preference)
        .bind(input.max_amount_usd_per_request.is_some())
        .bind(input.max_amount_usd_per_request.flatten())
        .bind(input.max_amount_usd_per_turn.is_some())
        .bind(input.max_amount_usd_per_turn.flatten())
        .bind(input.max_amount_usd_per_day.is_some())
        .bind(input.max_amount_usd_per_day.flatten())
        .bind(input.require_approval_above_usd.is_some())
        .bind(input.require_approval_above_usd.flatten())
        .bind(input.status)
        .bind(input.metadata)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    pub async fn list_payment_attempts(
        &self,
        org_id: i64,
        session_id: Option<Uuid>,
        limit: i64,
    ) -> Result<Vec<PaymentAttemptRow>> {
        let sql = format!(
            "SELECT {ATTEMPT_COLUMNS} FROM payment_attempts \
             WHERE org_id = $1 AND ($2::uuid IS NULL OR session_id = $2) \
             ORDER BY created_at DESC LIMIT $3"
        );
        Ok(
            sqlx::query_as::<_, PaymentAttemptRow>(sqlx::AssertSqlSafe(sql.as_str()))
                .bind(org_id)
                .bind(session_id)
                .bind(limit)
                .fetch_all(&self.pool)
                .await?,
        )
    }

    pub async fn create_payment_attempt(
        &self,
        org_id: i64,
        input: CreatePaymentAttemptRow,
    ) -> Result<PaymentAttemptRow> {
        let row = sqlx::query_as::<_, PaymentAttemptRow>(
            r#"
            INSERT INTO payment_attempts (
                org_id, payment_account_id, session_id, capability, operation, rail,
                amount_usd, currency, target_url, request_hash, status, error_message, receipt
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
            RETURNING id, org_id, payment_account_id, session_id, capability, operation,
                      rail, amount_usd, currency, target_url, request_hash, status,
                      error_message, receipt, created_at, updated_at
            "#,
        )
        .bind(org_id)
        .bind(input.payment_account_id)
        .bind(input.session_id)
        .bind(input.capability)
        .bind(input.operation)
        .bind(input.rail)
        .bind(input.amount_usd)
        .bind(input.currency)
        .bind(input.target_url)
        .bind(input.request_hash)
        .bind(input.status)
        .bind(input.error_message)
        .bind(input.receipt)
        .fetch_one(&self.pool)
        .await?;
        Ok(row)
    }
}
