use super::*;
#[async_trait]
impl ChannelStreamDelivery for SlackDeliveryAdapter {
    async fn start(&self, c: &ChannelDeliveryContext) -> Result<String, String> {
        self.start_checked(c)
            .await
            .map_err(|error| error.to_string())
    }
    async fn start_checked(
        &self,
        c: &ChannelDeliveryContext,
    ) -> Result<String, ChannelDeliveryResult> {
        self.surfaces()?.start_checked(c).await
    }
    async fn append(&self, h: &str, t: &str, c: &ChannelDeliveryContext) -> ChannelDeliveryResult {
        match self.surfaces() {
            Ok(surface) => surface.append(h, t, c).await,
            Err(error) => error,
        }
    }
    async fn replace(&self, h: &str, t: &str, c: &ChannelDeliveryContext) -> ChannelDeliveryResult {
        match self.surfaces() {
            Ok(surface) => surface.replace(h, t, c).await,
            Err(error) => error,
        }
    }
    async fn stop(&self, h: &str, c: &ChannelDeliveryContext) -> ChannelDeliveryResult {
        match self.surfaces() {
            Ok(surface) => surface.stop(h, c).await,
            Err(error) => error,
        }
    }
}
