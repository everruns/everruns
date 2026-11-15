use super::*;
use everruns_integrations::slack_channel::surfaces::SlackSurfaces;
pub(super) use everruns_integrations::slack_channel::surfaces::classify_slack_failure;
#[async_trait]
impl ChannelStreamDelivery for SlackDeliveryAdapter {
    async fn start(&self, c: &ChannelDeliveryContext) -> Result<String, String> {
        SlackSurfaces {
            api_base: &self.api_base,
        }
        .start(c)
        .await
    }
    async fn append(&self, h: &str, t: &str, c: &ChannelDeliveryContext) -> ChannelDeliveryResult {
        SlackSurfaces {
            api_base: &self.api_base,
        }
        .append(h, t, c)
        .await
    }
    async fn replace(&self, h: &str, t: &str, c: &ChannelDeliveryContext) -> ChannelDeliveryResult {
        SlackSurfaces {
            api_base: &self.api_base,
        }
        .replace(h, t, c)
        .await
    }
    async fn stop(&self, h: &str, c: &ChannelDeliveryContext) -> ChannelDeliveryResult {
        SlackSurfaces {
            api_base: &self.api_base,
        }
        .stop(h, c)
        .await
    }
}
