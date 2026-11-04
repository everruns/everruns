//! Trusted host's completed receipt projection under its pre-dispatch identity.
//!
//! Not wired to worker events or a public RPC. Caller is the credential-owning
//! host's admitted financial owner. Its transaction must validate the immutable
//! request/account binding and settle the reservation before commit; this journal
//! is not an authorization or financial ownership registry. No separate pool
//! fallback is allowed. Existing event listeners keep their legacy identity path.

use anyhow::{Result, bail};
use everruns_contracts::managed_dispatch::SupplierReceipt;
use sqlx::{Postgres, Transaction};

/// Insert once; reject a legacy/cross-owner collision instead of overwriting it.
/// Returns true only for a new row. Financial/session/reporting rollups belong
/// to the same host transaction and must only advance for a new receipt.
pub async fn insert_host_generation(
    tx: &mut Transaction<'_, Postgres>,
    org_id: i64,
    receipt: &SupplierReceipt,
) -> Result<bool> {
    if receipt.review().is_some() {
        bail!("review receipt cannot become a completed generation");
    }
    let identity = receipt.identity();
    let org_matches: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM organizations WHERE org_id=$1 AND public_id=$2)",
    )
    .bind(org_id)
    .bind(&identity.organization)
    .fetch_one(&mut **tx)
    .await?;
    if !org_matches {
        bail!("host generation organization mismatch");
    }
    let session_id = if let Some(id) = identity.session_id {
        let owner: Option<i64> = sqlx::query_scalar("SELECT org_id FROM sessions WHERE id=$1")
            .bind(id)
            .fetch_optional(&mut **tx)
            .await?;
        match owner {
            Some(owner) if owner != org_id => bail!("host generation session mismatch"),
            Some(_) => Some(id),
            // Deleting a conversation cannot erase already-incurred supplier
            // liability. The admitted host/financial receipt retains its scope.
            None => None,
        }
    } else {
        None
    };
    let provider_org: Option<(i64, bool)> =
        sqlx::query_as("SELECT org_id,managed FROM providers WHERE id=$1")
            .bind(identity.provider_id)
            .fetch_optional(&mut **tx)
            .await?;
    if provider_org.is_some_and(|(owner, managed)| owner != org_id || !managed) {
        bail!("host generation provider mismatch");
    }
    let (Some(input), Some(output), Some(nanos)) = (
        receipt.usage().input_tokens,
        receipt.usage().output_tokens,
        receipt.usage().cost_nanos,
    ) else {
        bail!("host generation usage missing");
    };
    let input = i64::try_from(input)?;
    let output = i64::try_from(output)?;
    let cost = nanos as f64 / 1_000_000_000.0;
    let provider_id = identity.provider_id.to_string();
    let inserted: Option<uuid::Uuid> = sqlx::query_scalar(
        "INSERT INTO llm_generations(id,org_id,session_id,model,provider,input_tokens,output_tokens,actual_cost_usd,provider_response_id,provider_config_id,managed,created_at) VALUES($1,$2,$3,$4,'openrouter',$5,$6,$7,$8,$9,TRUE,$10) ON CONFLICT(id) DO NOTHING RETURNING id",
    ).bind(identity.generation_id).bind(org_id).bind(session_id).bind(receipt.model())
        .bind(input).bind(output).bind(cost).bind(receipt.supplier_response_id())
        .bind(&provider_id).bind(identity.started_at).fetch_optional(&mut **tx).await?;
    if inserted.is_some() {
        return Ok(true);
    }
    let matches: bool = sqlx::query_scalar(
        "SELECT org_id=$2 AND session_id IS NOT DISTINCT FROM $3 AND model IS NOT DISTINCT FROM $4 AND provider='openrouter' AND input_tokens=$5 AND output_tokens=$6 AND actual_cost_usd IS NOT DISTINCT FROM $7 AND provider_response_id IS NOT DISTINCT FROM $8 AND provider_config_id IS NOT DISTINCT FROM $9 AND managed AND created_at=$10 AND event_id IS NULL FROM llm_generations WHERE id=$1",
    ).bind(identity.generation_id).bind(org_id).bind(session_id).bind(receipt.model())
        .bind(input).bind(output).bind(cost).bind(receipt.supplier_response_id())
        .bind(&provider_id).bind(identity.started_at).fetch_one(&mut **tx).await?;
    if !matches {
        bail!("host generation identity already belongs to another observation");
    }
    Ok(false)
}

#[cfg(test)]
mod tests;
