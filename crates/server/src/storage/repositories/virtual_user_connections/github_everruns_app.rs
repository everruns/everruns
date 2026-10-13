// PostgreSQL repository: the shared Everruns GitHub App's org links (D3 in
// knowledge/integrations/github-model.md).
//
// Decision: an org link is an organization connection (owner_scope
// 'organization', provider 'github', connection_type 'everruns_app') carried by
// its own hidden organization virtual user, like sandbox and search accounts.
// An agent that chose the link holds a `github` row of the same type on its
// service virtual user, with the same `installation_id`. Those agent rows are
// found by installation id within the org, so unlinking (by an admin or by
// GitHub's `installation.deleted`) removes them in the same transaction:
// nothing keeps minting for an installation the org no longer links.

use super::Database;
use super::rows::VirtualUserConnectionRow;
use crate::github_apps::{EVERRUNS_APP_CONNECTION_TYPE, GITHUB_PROVIDER};
use anyhow::Result;
use everruns_contracts::typed_id::VirtualUserId;
use everruns_server_macros::sql;

/// An org link found by installation id alone, for the unauthenticated webhook.
#[derive(Debug, Clone)]
pub struct EverrunsAppOrgLink {
    pub org_id: i64,
    pub row: VirtualUserConnectionRow,
}

impl Database {
    /// The org's linked shared-App installations, by account name.
    pub async fn list_everruns_app_links(
        &self,
        org_id: i64,
    ) -> Result<Vec<VirtualUserConnectionRow>> {
        Ok(sqlx::query_as::<_, VirtualUserConnectionRow>(sql!(
            r#"
            SELECT {VirtualUserConnectionRow}
            FROM virtual_user_connections
            WHERE virtual_user_id IN (
                SELECT id FROM virtual_users WHERE org_id = $1 AND usage = 'organization'
            ) AND owner_scope = 'organization' AND provider = $2 AND connection_type = $3
            ORDER BY lower(name), created_at
        "#
        ))
        .bind(org_id)
        .bind(GITHUB_PROVIDER)
        .bind(EVERRUNS_APP_CONNECTION_TYPE)
        .fetch_all(&self.pool)
        .await?)
    }

    /// One link of the org, by its connection id.
    pub async fn get_everruns_app_link(
        &self,
        org_id: i64,
        connection_id: uuid::Uuid,
    ) -> Result<Option<VirtualUserConnectionRow>> {
        Ok(sqlx::query_as::<_, VirtualUserConnectionRow>(sql!(
            r#"
            SELECT {VirtualUserConnectionRow}
            FROM virtual_user_connections
            WHERE virtual_user_id IN (
                SELECT id FROM virtual_users WHERE org_id = $1 AND usage = 'organization'
            ) AND owner_scope = 'organization' AND provider = $2 AND connection_type = $3
              AND id = $4
            LIMIT 1
        "#
        ))
        .bind(org_id)
        .bind(GITHUB_PROVIDER)
        .bind(EVERRUNS_APP_CONNECTION_TYPE)
        .bind(connection_id)
        .fetch_optional(&self.pool)
        .await?)
    }

    /// The link holding `installation_id`, in whichever org. Used by the
    /// shared App's webhook, which only knows the installation, and by the
    /// link flow to refuse an installation another org already holds.
    pub async fn find_everruns_app_link(
        &self,
        installation_id: i64,
    ) -> Result<Option<EverrunsAppOrgLink>> {
        let row = sqlx::query_as::<_, VirtualUserConnectionRow>(sql!(
            r#"
            SELECT {VirtualUserConnectionRow}
            FROM virtual_user_connections
            WHERE owner_scope = 'organization' AND provider = $1 AND installation_id = $2
            LIMIT 1
        "#
        ))
        .bind(GITHUB_PROVIDER)
        .bind(installation_id)
        .fetch_optional(&self.pool)
        .await?;
        let Some(row) = row else {
            return Ok(None);
        };
        let org_id: Option<i64> = sqlx::query_scalar(
            "SELECT org_id FROM virtual_users WHERE id = $1 AND usage = 'organization'",
        )
        .bind(row.virtual_user_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(org_id.map(|org_id| EverrunsAppOrgLink { org_id, row }))
    }

    /// Service virtual users in the org whose `github` row chose the link for
    /// `installation_id`: the agents a shared-App delivery may reach.
    pub async fn list_everruns_app_identities(
        &self,
        org_id: i64,
        installation_id: i64,
    ) -> Result<Vec<VirtualUserId>> {
        Ok(sqlx::query_scalar(
            r#"SELECT c.virtual_user_id
               FROM virtual_user_connections c
               JOIN virtual_users v ON v.id = c.virtual_user_id
               WHERE v.org_id = $1 AND v.usage = 'service' AND v.status = 'active'
                 AND c.provider = $2 AND c.connection_type = $3 AND c.installation_id = $4"#,
        )
        .bind(org_id)
        .bind(GITHUB_PROVIDER)
        .bind(EVERRUNS_APP_CONNECTION_TYPE)
        .bind(installation_id)
        .fetch_all(&self.pool)
        .await?)
    }

    /// Remove an org link and every agent row in the org that chose it.
    /// `None` when the org holds no such link; otherwise how many agent rows
    /// went with it.
    pub async fn unlink_everruns_app(
        &self,
        org_id: i64,
        connection_id: uuid::Uuid,
    ) -> Result<Option<u64>> {
        let mut tx = self.pool.begin().await?;
        let link: Option<(VirtualUserId, Option<i64>)> = sqlx::query_as(
            r#"SELECT c.virtual_user_id, c.installation_id FROM virtual_user_connections c
               JOIN virtual_users v ON v.id = c.virtual_user_id
               WHERE c.id = $1 AND v.org_id = $2 AND v.usage = 'organization'
                 AND c.owner_scope = 'organization' AND c.provider = $3
                 AND c.connection_type = $4
               FOR UPDATE OF c"#,
        )
        .bind(connection_id)
        .bind(org_id)
        .bind(GITHUB_PROVIDER)
        .bind(EVERRUNS_APP_CONNECTION_TYPE)
        .fetch_optional(&mut *tx)
        .await?;
        let Some((owner, installation_id)) = link else {
            return Ok(None);
        };
        let agents = sqlx::query(
            r#"DELETE FROM virtual_user_connections c
               USING virtual_users v
               WHERE v.id = c.virtual_user_id AND v.org_id = $1 AND v.usage = 'service'
                 AND c.provider = $2 AND c.connection_type = $3 AND c.installation_id = $4"#,
        )
        .bind(org_id)
        .bind(GITHUB_PROVIDER)
        .bind(EVERRUNS_APP_CONNECTION_TYPE)
        .bind(installation_id)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        sqlx::query("DELETE FROM virtual_user_connections WHERE id = $1")
            .bind(connection_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM virtual_users WHERE id = $1 AND usage = 'organization'")
            .bind(owner)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(Some(agents))
    }

    /// Record the permissions GitHub reports for an installation on the org
    /// link and on the agent rows that chose it.
    pub async fn update_everruns_app_scopes(
        &self,
        org_id: i64,
        installation_id: i64,
        scopes: Option<&str>,
    ) -> Result<u64> {
        Ok(sqlx::query(
            r#"UPDATE virtual_user_connections c SET scopes = $4, updated_at = NOW()
               FROM virtual_users v
               WHERE v.id = c.virtual_user_id AND v.org_id = $1
                 AND v.usage IN ('organization', 'service')
                 AND c.provider = $2 AND c.connection_type = $3 AND c.installation_id = $5"#,
        )
        .bind(org_id)
        .bind(GITHUB_PROVIDER)
        .bind(EVERRUNS_APP_CONNECTION_TYPE)
        .bind(scopes)
        .bind(installation_id)
        .execute(&self.pool)
        .await?
        .rows_affected())
    }
}
