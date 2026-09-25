use super::types::PluginInstallRow;
use crate::domains::common::{CommandError, Ctx, classify_anyhow};
use crate::kernel_imports::everruns_provider::typed_id::PluginInstallId;

pub(super) fn parse_plugin_public_id(id: &str) -> Result<PluginInstallId, CommandError> {
    id.parse::<PluginInstallId>()
        .map_err(|error| CommandError::bad_request(format!("Invalid plugin ID: {error}")))
}

pub(super) async fn get_install_by_public_id(
    ctx: &Ctx,
    id: &str,
) -> Result<PluginInstallRow, CommandError> {
    let public_id = parse_plugin_public_id(id)?;
    ctx.db
        .get_plugin_install_by_public_id(ctx.org_id(), &public_id.to_string())
        .await
        .map_err(classify_anyhow)?
        .ok_or_else(|| CommandError::not_found("Installed plugin"))
}
