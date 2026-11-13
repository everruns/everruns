//! Native extraction transports; capabilities are checked before network I/O.
use crate::{
    error::{AgentLoopError, Result},
    extractions::{ExtractionAssetFormat, ExtractionOutcome, ExtractionRequest, ExtractionType},
};
use async_trait::async_trait;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExtractionDriverCapabilities {
    pub field_types: Vec<ExtractionType>,
    pub images: bool,
    pub documents: bool,
    pub max_fields: Option<usize>,
    pub max_assets: Option<usize>,
    pub max_state_bytes: Option<usize>,
    /// Encoded asset bytes, across the complete request.
    pub max_asset_bytes: Option<usize>,
}
impl ExtractionDriverCapabilities {
    pub fn all() -> Self {
        use ExtractionType::*;
        Self {
            field_types: vec![
                Text, Number, Date, Time, Datetime, Duration, List, Estimate, Forecast,
            ],
            images: true,
            documents: true,
            ..Self::default()
        }
    }
    pub fn check(&self, _driver: &str, request: &ExtractionRequest) -> Result<()> {
        if request
            .fields
            .values()
            .any(|field| !self.field_types.contains(&field.kind))
        {
            return Err(AgentLoopError::llm(
                "extraction driver does not support requested field type",
            ));
        }
        if self.max_fields.is_some_and(|n| request.fields.len() > n)
            || self.max_assets.is_some_and(|n| request.assets.len() > n)
            || self
                .max_state_bytes
                .is_some_and(|n| request.state.len() > n)
        {
            return Err(AgentLoopError::llm(
                "extraction request exceeds driver limits",
            ));
        }
        let mut bytes = 0usize;
        for asset in request.assets.values() {
            let supported = if asset.format == ExtractionAssetFormat::Pdf {
                self.documents
            } else {
                self.images
            };
            if !supported {
                return Err(AgentLoopError::llm(
                    "extraction driver does not support this asset format",
                ));
            }
            bytes = bytes.saturating_add(asset.data_base64.len());
        }
        if self.max_asset_bytes.is_some_and(|n| bytes > n) {
            return Err(AgentLoopError::llm(
                "extraction assets exceed driver size limit",
            ));
        }
        // Decode/validate asset contents only after cheap size limits pass.
        request.validate()?;
        Ok(())
    }
}
#[async_trait]
pub trait ExtractionDriver: Send + Sync {
    fn id(&self) -> &str;
    fn capabilities(&self) -> ExtractionDriverCapabilities;
    async fn extract(
        &self,
        endpoint: &crate::runtime_provider::ProviderEndpoint,
        request: ExtractionRequest,
    ) -> Result<ExtractionOutcome>;
    async fn list_models(
        &self,
        _endpoint: &crate::runtime_provider::ProviderEndpoint,
    ) -> Result<Option<Vec<crate::model_discovery::DiscoveredModel>>> {
        Ok(None)
    }
}
