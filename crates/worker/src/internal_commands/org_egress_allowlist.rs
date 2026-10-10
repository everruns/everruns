//! `OrgEgressAllowlist` over the `worker_get_org_egress_allowlist` command.
//!
//! A distributed worker has no database, so it asks the control plane for an
//! org's enforced allowlist extension. The egress boundary caches each answer
//! (`ORG_EGRESS_ALLOWLIST_CACHE_TTL`, 60 seconds) and only asks for a
//! request the system policy would otherwise refuse, so this is not on the hot
//! path. Any failure is returned as an error, which the boundary treats as "no
//! extension" (fail closed).

use super::InternalCommandTransport;
use async_trait::async_trait;
use everruns_contracts::runtime::network_access::NetworkAccessList;
use everruns_contracts::runtime::organization::org_internal_id_from_public;
use everruns_contracts::runtime::{OrgEgressAllowlist, org_egress_extension};
use everruns_contracts::typed_id::OrgId;
use std::sync::Arc;

const COMMAND: &str = "worker_get_org_egress_allowlist";

/// The org allowlist resolver a worker installs for its runtime egress.
pub struct CommandOrgEgressAllowlist<T> {
    /// Builds a transport acting as the given org's internal caller.
    transport_for: Arc<dyn Fn(i64) -> T + Send + Sync>,
}

impl<T: InternalCommandTransport> CommandOrgEgressAllowlist<T> {
    pub fn new(transport_for: impl Fn(i64) -> T + Send + Sync + 'static) -> Self {
        Self {
            transport_for: Arc::new(transport_for),
        }
    }
}

#[derive(serde::Deserialize)]
struct Enforced {
    patterns: Vec<String>,
}

#[async_trait]
impl<T: InternalCommandTransport + 'static> OrgEgressAllowlist for CommandOrgEgressAllowlist<T> {
    async fn extension(&self, org_id: &OrgId) -> Result<Option<NetworkAccessList>, String> {
        let transport = (self.transport_for)(org_internal_id_from_public(*org_id));
        match transport
            .execute_internal_command(COMMAND, serde_json::json!({}))
            .await
        {
            Ok(Ok(value)) => {
                let enforced: Enforced =
                    serde_json::from_value(value).map_err(|error| error.to_string())?;
                // The server returns patterns only for a granted org.
                Ok(org_egress_extension(true, &enforced.patterns))
            }
            Ok(Err(error)) => Err(error.message),
            Err(error) => Err(error.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_contracts::error::Result;
    use everruns_internal_protocol::proto;
    use serde_json::Value;
    use std::sync::Mutex;

    struct Recorded {
        calls: Arc<Mutex<Vec<(i64, String)>>>,
        org_id: i64,
        answer: std::result::Result<Value, String>,
    }

    #[async_trait]
    impl InternalCommandTransport for Recorded {
        async fn execute_internal_command(
            &self,
            name: &str,
            _params: Value,
        ) -> Result<std::result::Result<Value, proto::CommandError>> {
            self.calls
                .lock()
                .unwrap()
                .push((self.org_id, name.to_string()));
            Ok(self.answer.clone().map_err(|message| proto::CommandError {
                message,
                ..Default::default()
            }))
        }
    }

    type Calls = Arc<Mutex<Vec<(i64, String)>>>;

    fn resolver(
        answer: std::result::Result<Value, String>,
    ) -> (CommandOrgEgressAllowlist<Recorded>, Calls) {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let recorded = calls.clone();
        let resolver = CommandOrgEgressAllowlist::new(move |org_id| Recorded {
            calls: recorded.clone(),
            org_id,
            answer: answer.clone(),
        });
        (resolver, calls)
    }

    #[tokio::test]
    async fn asks_the_control_plane_as_the_requests_org() {
        let (resolver, calls) = resolver(Ok(serde_json::json!({ "patterns": ["api.acme.com"] })));
        let org: OrgId = "org_0000000000000000000000000000002a".parse().unwrap();
        let extension = resolver.extension(&org).await.unwrap().expect("granted");
        assert!(extension.is_url_allowed("https://api.acme.com/v1"));
        assert_eq!(*calls.lock().unwrap(), vec![(42, COMMAND.to_string())]);
    }

    #[tokio::test]
    async fn no_patterns_is_no_extension_and_failures_are_errors() {
        let org: OrgId = "org_00000000000000000000000000000001".parse().unwrap();
        let (empty, _) = resolver(Ok(serde_json::json!({ "patterns": [] })));
        assert_eq!(empty.extension(&org).await, Ok(None));
        let (failing, _) = resolver(Err("Internal command".into()));
        assert!(failing.extension(&org).await.is_err());
    }
}
