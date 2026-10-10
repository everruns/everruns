// PostgreSQL repository: machine payment accounts, policies, and attempts.

pub(super) mod rows;
use rows::*;

use super::Database;
use anyhow::Result;
use everruns_server_macros::sql;
use uuid::Uuid;

const ACCOUNT_COLUMNS: &str = "id, org_id, owner_type, owner_id, rail, label, public_address, credential_encrypted, status, metadata, created_at, updated_at";
const POLICY_COLUMNS: &str = "id, org_id, payment_account_id, subject_type, subject_id, allowed_capabilities, allowed_hosts, rail_preference, max_amount_usd_per_request, max_amount_usd_per_turn, max_amount_usd_per_day, require_approval_above_usd, status, metadata, created_at, updated_at";
const ATTEMPT_COLUMNS: &str = "id, org_id, payment_account_id, session_id, capability, operation, rail, amount_usd, currency, target_url, request_hash, status, error_message, receipt, payment_policy_id, input_message_id, created_at, updated_at";

impl Database {
    pub async fn create_payment_account(
        &self,
        org_id: i64,
        input: CreatePaymentAccountRow,
    ) -> Result<PaymentAccountRow> {
        let row = sqlx::query_as::<_, PaymentAccountRow>(
            sql!(r#"
            INSERT INTO payment_accounts
                (org_id, owner_type, owner_id, rail, label, public_address, credential_encrypted, metadata)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
            RETURNING {PaymentAccountRow}
            "#),
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
        let row = sqlx::query_as::<_, PaymentAccountRow>(sql!(
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
            RETURNING {PaymentAccountRow}
            "#
        ))
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
        let row = sqlx::query_as::<_, PaymentPolicyRow>(sql!(
            r#"
            INSERT INTO payment_policies (
                org_id, payment_account_id, subject_type, subject_id,
                allowed_capabilities, allowed_hosts, rail_preference,
                max_amount_usd_per_request, max_amount_usd_per_turn,
                max_amount_usd_per_day, require_approval_above_usd, metadata
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
            RETURNING {PaymentPolicyRow}
            "#
        ))
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
            sql!(r#"
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
            RETURNING {PaymentPolicyRow}
            "#),
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
        let row = sqlx::query_as::<_, PaymentAttemptRow>(sql!(
            r#"
            INSERT INTO payment_attempts (
                org_id, payment_account_id, session_id, capability, operation, rail,
                amount_usd, currency, target_url, request_hash, status, error_message, receipt,
                payment_policy_id, input_message_id
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)
            RETURNING {PaymentAttemptRow}
            "#
        ))
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
        .bind(input.payment_policy_id)
        .bind(input.input_message_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(row)
    }

    /// Reserve spend against a policy's per-turn and per-day windows by inserting
    /// a `pending` attempt, or report which window it would exceed.
    ///
    /// THREAT[TM-AGENT-022]: parallel paid tool calls could each pass a read-only
    /// limit check and together overspend. The policy row is locked for the
    /// check-and-insert, so reservations against one policy serialize. Pending and
    /// succeeded attempts count toward the windows; failed and released ones do not.
    /// The day window is a rolling 24 hours. A per-turn limit with no turn binding
    /// fails closed (`input_message_id` is required to attribute spend to a turn).
    pub async fn reserve_payment_attempt(
        &self,
        org_id: i64,
        policy_id: Uuid,
        limits: PaymentSpendLimits,
        input: CreatePaymentAttemptRow,
    ) -> Result<PaymentReservation> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("SELECT 1 FROM payment_policies WHERE org_id = $1 AND id = $2 FOR UPDATE")
            .bind(org_id)
            .bind(policy_id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or_else(|| anyhow::anyhow!("payment policy {policy_id} not found"))?;

        if let Some(limit) = limits.max_amount_usd_per_turn {
            let Some(input_message_id) = input.input_message_id else {
                return Ok(PaymentReservation::LimitExceeded {
                    window: "turn",
                    limit_usd: limit,
                    spent_usd: 0.0,
                });
            };
            let spent: f64 = sqlx::query_scalar(
                "SELECT COALESCE(SUM(amount_usd), 0)::float8 FROM payment_attempts \
                 WHERE org_id = $1 AND payment_policy_id = $2 AND input_message_id = $3 \
                   AND status IN ('pending', 'succeeded')",
            )
            .bind(org_id)
            .bind(policy_id)
            .bind(input_message_id)
            .fetch_one(&mut *tx)
            .await?;
            if spent + input.amount_usd > limit {
                return Ok(PaymentReservation::LimitExceeded {
                    window: "turn",
                    limit_usd: limit,
                    spent_usd: spent,
                });
            }
        }
        if let Some(limit) = limits.max_amount_usd_per_day {
            let spent: f64 = sqlx::query_scalar(
                "SELECT COALESCE(SUM(amount_usd), 0)::float8 FROM payment_attempts \
                 WHERE org_id = $1 AND payment_policy_id = $2 \
                   AND created_at > NOW() - INTERVAL '24 hours' \
                   AND status IN ('pending', 'succeeded')",
            )
            .bind(org_id)
            .bind(policy_id)
            .fetch_one(&mut *tx)
            .await?;
            if spent + input.amount_usd > limit {
                return Ok(PaymentReservation::LimitExceeded {
                    window: "day",
                    limit_usd: limit,
                    spent_usd: spent,
                });
            }
        }

        let row = sqlx::query_as::<_, PaymentAttemptRow>(sql!(
            r#"
            INSERT INTO payment_attempts (
                org_id, payment_account_id, session_id, capability, operation, rail,
                amount_usd, currency, target_url, request_hash, status, error_message, receipt,
                payment_policy_id, input_message_id
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, 'pending', NULL, $11, $12, $13)
            RETURNING {PaymentAttemptRow}
            "#
        ))
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
        .bind(input.receipt)
        .bind(policy_id)
        .bind(input.input_message_id)
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(PaymentReservation::Reserved(Box::new(row)))
    }

    /// Settle a reserved attempt with its final status and receipt.
    pub async fn finish_payment_attempt(
        &self,
        org_id: i64,
        attempt_id: Uuid,
        status: &str,
        error_message: Option<String>,
        receipt: serde_json::Value,
    ) -> Result<PaymentAttemptRow> {
        let row = sqlx::query_as::<_, PaymentAttemptRow>(sql!(
            r#"
            UPDATE payment_attempts
            SET status = $3, error_message = $4, receipt = $5
            WHERE org_id = $1 AND id = $2
            RETURNING {PaymentAttemptRow}
            "#
        ))
        .bind(org_id)
        .bind(attempt_id)
        .bind(status)
        .bind(error_message)
        .bind(receipt)
        .fetch_one(&self.pool)
        .await?;
        Ok(row)
    }
}
