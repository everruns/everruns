// A default data plane for new orgs: how a self-hosted install runs data
// plane v2 by default (everruns k8s up, the Compose file, `everruns docker
// up`), while v1 stays one setting away.
//
// Decisions:
// - Configured by the environment: `EVERRUNS_DEFAULT_DATA_PLANE_URL` is the
//   data plane's gateway (a node of its directory shard) and
//   `EVERRUNS_DEFAULT_DATA_PLANE` its id (`local` when unset). Empty URL: no
//   default, every org runs on v1 as before. `EVERRUNS_DATA_PLANE=v1` turns
//   it off too, so an install switches with one variable.
// - The server registers that data plane itself at start, so an install
//   needs no operator step (`everruns-dataplane register` stays for
//   production, where planes are added by hand).
// - It takes only orgs that have never had a session. An org with v1
//   sessions keeps them on v1: their history lives in PostgreSQL and moving
//   it is a separate step (org moves, data plane v2 finalization). So
//   turning v2 on in an existing install moves its new orgs and leaves the
//   ones in use alone.
// - `EVERRUNS_DEFAULT_DATA_PLANE_SINCE` (RFC 3339) narrows that to orgs
//   created at or after the time: a service with many orgs that signed up
//   and never ran an agent moves only the ones that sign up from now on. A
//   value that does not parse turns the default off rather than place every
//   empty org.
// - A poll, not a hook on every org-creation path (sign-up, invitations,
//   the default org at start, the organizations API): one query every
//   few seconds catches them all, before a person can set up a provider and
//   start a session. The organizations API also places an org at creation
//   (`DefaultDataPlaneInitializer`), so a script that creates an org and
//   starts a session at once lands on v2 too.

use std::time::Duration;

use super::{ControlPlane, enable_org};

/// How often orgs are checked.
const EVERY: Duration = Duration::from_secs(3);
/// Orgs placed per check.
const BATCH: i64 = 100;

/// The data plane new orgs run on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DefaultDataPlane {
    pub id: String,
    pub url: String,
    /// Only orgs created at or after this time; every org when unset.
    pub since: Option<chrono::DateTime<chrono::Utc>>,
}

impl DefaultDataPlane {
    pub fn from_env() -> Option<Self> {
        let plane = Self::from_vars(|name| std::env::var(name).ok());
        if plane.is_none()
            && std::env::var("EVERRUNS_DEFAULT_DATA_PLANE_URL").is_ok_and(|u| !u.trim().is_empty())
            && !std::env::var("EVERRUNS_DATA_PLANE")
                .is_ok_and(|v| v.trim().eq_ignore_ascii_case("v1"))
        {
            tracing::warn!(
                "EVERRUNS_DEFAULT_DATA_PLANE_URL is set but EVERRUNS_DP_KEY or EVERRUNS_DP_TOKEN \
                 is not: new organizations run on v1"
            );
        }
        plane
    }

    fn from_vars(var: impl Fn(&str) -> Option<String>) -> Option<Self> {
        if var("EVERRUNS_DATA_PLANE").is_some_and(|v| v.trim().eq_ignore_ascii_case("v1")) {
            return None;
        }
        // Without the link's secrets no organization could be placed (its
        // data key is wrapped with the key): an install that upgraded its
        // Compose file but not its .env keeps running v1.
        let set = |name: &str| var(name).is_some_and(|v| !v.trim().is_empty());
        if !set("EVERRUNS_DP_KEY") || !set("EVERRUNS_DP_TOKEN") {
            return None;
        }
        let url = var("EVERRUNS_DEFAULT_DATA_PLANE_URL")?;
        let url = url.trim().trim_end_matches('/');
        if url.is_empty() {
            return None;
        }
        let id = var("EVERRUNS_DEFAULT_DATA_PLANE")
            .map(|id| id.trim().to_owned())
            .filter(|id| !id.is_empty())
            .unwrap_or_else(|| "local".to_owned());
        let since = match var("EVERRUNS_DEFAULT_DATA_PLANE_SINCE")
            .map(|v| v.trim().to_owned())
            .filter(|v| !v.is_empty())
        {
            None => None,
            Some(value) => match chrono::DateTime::parse_from_rfc3339(&value) {
                Ok(at) => Some(at.with_timezone(&chrono::Utc)),
                Err(error) => {
                    tracing::warn!(
                        %error,
                        "EVERRUNS_DEFAULT_DATA_PLANE_SINCE is not an RFC 3339 time: \
                         no default data plane"
                    );
                    return None;
                }
            },
        };
        Some(Self {
            id,
            url: url.to_owned(),
            since,
        })
    }
}

/// Register `plane` and put every org it should take on it.
pub async fn place_orgs(cp: &ControlPlane, plane: &DefaultDataPlane) -> anyhow::Result<usize> {
    let mut placed = 0;
    for org_id in cp.db.dp_orgs_to_place(BATCH, plane.since).await? {
        enable_org(cp, org_id, &plane.id).await?;
        tracing::info!(org_id, data_plane = %plane.id, "org placed on the default data plane");
        placed += 1;
    }
    Ok(placed)
}

/// Register `plane`, then keep placing new orgs on it.
pub async fn run(cp: ControlPlane, plane: DefaultDataPlane) {
    loop {
        match cp
            .db
            .dp_upsert_data_plane(&plane.id, &plane.url, &plane.url)
            .await
        {
            Ok(()) => break,
            Err(error) => {
                tracing::warn!(%error, "default data plane: registration failed; retrying");
                tokio::time::sleep(EVERY).await;
            }
        }
    }
    tracing::info!(data_plane = %plane.id, url = %plane.url, "default data plane for new orgs");
    loop {
        if let Err(error) = place_orgs(&cp, &plane).await {
            tracing::warn!(%error, "default data plane: placing orgs failed");
        }
        tokio::time::sleep(EVERY).await;
    }
}

/// The organizations API's initializer for the configured default data
/// plane, when there is one.
pub fn initializer() -> Option<std::sync::Arc<dyn crate::setup::org_init::OrgInitializer>> {
    let plane = DefaultDataPlane::from_env()?;
    Some(std::sync::Arc::new(DefaultDataPlaneInitializer { plane }))
}

/// Places an org created through the organizations API at once.
pub struct DefaultDataPlaneInitializer {
    pub plane: DefaultDataPlane,
}

#[async_trait::async_trait]
impl crate::setup::org_init::OrgInitializer for DefaultDataPlaneInitializer {
    async fn on_org_created(
        &self,
        ctx: crate::setup::org_init::OrgInitContext<'_>,
    ) -> anyhow::Result<()> {
        let db = std::sync::Arc::new(ctx.db.clone());
        let cp = ControlPlane::from_env(db);
        cp.db
            .dp_upsert_data_plane(&self.plane.id, &self.plane.url, &self.plane.url)
            .await?;
        enable_org(&cp, ctx.org_id, &self.plane.id).await?;
        Ok(())
    }

    /// The poll places it anyway if this fails.
    fn required(&self) -> bool {
        false
    }

    fn name(&self) -> &str {
        "default-data-plane"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LINK: [(&str, &str); 2] = [("EVERRUNS_DP_KEY", "a2V5"), ("EVERRUNS_DP_TOKEN", "token")];

    fn plane(vars: &[(&str, &str)]) -> Option<DefaultDataPlane> {
        plane_with(&[&LINK[..], vars].concat())
    }

    fn plane_with(vars: &[(&str, &str)]) -> Option<DefaultDataPlane> {
        DefaultDataPlane::from_vars(|name| {
            vars.iter()
                .find(|(k, _)| *k == name)
                .map(|(_, v)| (*v).to_owned())
        })
    }

    #[test]
    fn a_url_names_the_default_data_plane() {
        assert_eq!(
            plane(&[("EVERRUNS_DEFAULT_DATA_PLANE_URL", "http://shard:7000/")]),
            Some(DefaultDataPlane {
                id: "local".into(),
                url: "http://shard:7000".into(),
                since: None,
            })
        );
        assert_eq!(
            plane(&[
                ("EVERRUNS_DEFAULT_DATA_PLANE_URL", "http://shard:7000"),
                ("EVERRUNS_DEFAULT_DATA_PLANE", "dp-1"),
            ])
            .map(|p| p.id),
            Some("dp-1".into())
        );
    }

    #[test]
    fn no_url_or_v1_means_no_default() {
        assert_eq!(plane(&[]), None);
        assert_eq!(plane(&[("EVERRUNS_DEFAULT_DATA_PLANE_URL", " ")]), None);
        assert_eq!(
            plane(&[
                ("EVERRUNS_DEFAULT_DATA_PLANE_URL", "http://shard:7000"),
                ("EVERRUNS_DATA_PLANE", "V1"),
            ]),
            None
        );
    }

    #[test]
    fn a_cutoff_limits_placement_to_orgs_created_after_it() {
        let url = ("EVERRUNS_DEFAULT_DATA_PLANE_URL", "http://shard:7000");
        let at = plane(&[
            url,
            ("EVERRUNS_DEFAULT_DATA_PLANE_SINCE", "2026-11-19T01:00:00Z"),
        ])
        .and_then(|p| p.since);
        assert_eq!(
            at.map(|t| t.to_rfc3339()),
            Some("2026-11-19T01:00:00+00:00".into())
        );
        // A cutoff that does not parse never widens placement to every org.
        assert_eq!(
            plane(&[url, ("EVERRUNS_DEFAULT_DATA_PLANE_SINCE", "tomorrow")]),
            None
        );
        assert_eq!(
            plane(&[url, ("EVERRUNS_DEFAULT_DATA_PLANE_SINCE", " ")]).map(|p| p.since),
            Some(None)
        );
    }

    #[test]
    fn without_the_link_secrets_there_is_no_default() {
        let url = ("EVERRUNS_DEFAULT_DATA_PLANE_URL", "http://shard:7000");
        assert_eq!(plane_with(&[url]), None);
        assert_eq!(plane_with(&[url, LINK[0]]), None);
        assert_eq!(plane_with(&[url, LINK[1]]), None);
        assert_eq!(plane_with(&[url, LINK[0], ("EVERRUNS_DP_TOKEN", "")]), None);
        assert!(plane_with(&[url, LINK[0], LINK[1]]).is_some());
    }
}
