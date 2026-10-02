// AWS default credential chain (feature `default-credentials`).
//
// Decisions:
// - Opt-in only. Server and worker paths build Bedrock from org-scoped stored
//   keys (`BedrockAuth::from_config`) and must never fall back to ambient AWS
//   credentials; `from_driver_config` and the credential schema are unchanged.
//   Standalone hosts (a serve app on Amazon Bedrock AgentCore Runtime, an ECS
//   task, an EC2 box with an instance profile) opt in to run on the role the
//   platform already gives them.
// - aws-config builds the chain asynchronously, but provider assembly here is
//   sync. `LazyDefaultChain` defers `DefaultCredentialsChain::build()` to the
//   first credential request (always inside the SDK's async call path) and
//   keeps it in a `OnceCell`, so the constructor stays sync and does no I/O.
//   The SDK's identity cache sits in front of it and refreshes before expiry.
// - Region: explicit, else `AWS_REGION`, else `AWS_DEFAULT_REGION`, else
//   us-east-1, matching the static-key path's default. Region is not a secret,
//   so reading it here does not cross the provider credential contract.

use aws_config::default_provider::credentials::DefaultCredentialsChain;
use aws_credential_types::provider::{ProvideCredentials, future};
use aws_sdk_bedrockruntime::Client;
use aws_sdk_bedrockruntime::config::{BehaviorVersion, Builder as BedrockConfigBuilder, Region};
use tokio::sync::OnceCell;

use crate::credential::DEFAULT_REGION;

/// Credentials provider that builds the AWS default chain on first use.
#[derive(Debug)]
struct LazyDefaultChain {
    region: Region,
    chain: OnceCell<DefaultCredentialsChain>,
}

impl LazyDefaultChain {
    fn new(region: Region) -> Self {
        Self {
            region,
            chain: OnceCell::new(),
        }
    }
}

impl ProvideCredentials for LazyDefaultChain {
    fn provide_credentials<'a>(&'a self) -> future::ProvideCredentials<'a>
    where
        Self: 'a,
    {
        future::ProvideCredentials::new(async move {
            let chain = self
                .chain
                .get_or_init(|| {
                    DefaultCredentialsChain::builder()
                        .region(self.region.clone())
                        .build()
                })
                .await;
            chain.provide_credentials().await
        })
    }
}

/// Pick the region: explicit, else `AWS_REGION`, else `AWS_DEFAULT_REGION`,
/// else us-east-1. Empty values count as unset. `lookup` is injectable so tests
/// never mutate the process environment.
pub(crate) fn resolve_region<F>(explicit: Option<String>, lookup: F) -> String
where
    F: Fn(&str) -> Option<String>,
{
    let non_empty = |value: Option<String>| value.filter(|value| !value.trim().is_empty());
    non_empty(explicit)
        .or_else(|| non_empty(lookup("AWS_REGION")))
        .or_else(|| non_empty(lookup("AWS_DEFAULT_REGION")))
        .unwrap_or_else(|| DEFAULT_REGION.to_string())
}

/// Build a Bedrock runtime client whose credentials come from the AWS default
/// chain. No I/O happens until the first request.
pub(crate) fn build_client(region: String) -> Client {
    let region = Region::new(region);
    let config = BedrockConfigBuilder::new()
        .behavior_version(BehaviorVersion::latest())
        .credentials_provider(LazyDefaultChain::new(region.clone()))
        .region(region)
        .build();
    Client::from_conf(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lookup(vars: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
        move |name| {
            vars.iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| value.to_string())
        }
    }

    #[test]
    fn explicit_region_wins_over_the_environment() {
        let region = resolve_region(
            Some("eu-central-1".into()),
            lookup(&[("AWS_REGION", "us-west-2")]),
        );
        assert_eq!(region, "eu-central-1");
    }

    #[test]
    fn environment_region_order_is_aws_region_then_default_region() {
        let both = lookup(&[
            ("AWS_REGION", "us-west-2"),
            ("AWS_DEFAULT_REGION", "eu-west-1"),
        ]);
        assert_eq!(resolve_region(None, both), "us-west-2");
        let fallback = lookup(&[("AWS_DEFAULT_REGION", "eu-west-1")]);
        assert_eq!(resolve_region(None, fallback), "eu-west-1");
    }

    #[test]
    fn empty_values_fall_through_to_us_east_1() {
        let empty = lookup(&[("AWS_REGION", ""), ("AWS_DEFAULT_REGION", " ")]);
        assert_eq!(resolve_region(Some(String::new()), empty), "us-east-1");
        assert_eq!(resolve_region(None, lookup(&[])), "us-east-1");
    }

    #[tokio::test]
    async fn client_is_built_without_io_and_carries_the_region() {
        let client = build_client("ap-southeast-2".into());
        assert_eq!(
            client.config().region().map(|region| region.as_ref()),
            Some("ap-southeast-2")
        );
    }

    #[test]
    fn the_chain_is_not_built_until_credentials_are_requested() {
        let lazy = LazyDefaultChain::new(Region::new("us-east-1"));
        assert!(lazy.chain.get().is_none());
    }
}
