use super::*;

pub(super) async fn validate_service_account(
    ctx: &Ctx,
    id: Option<everruns_provider::typed_id::VirtualUserId>,
) -> Result<(), CommandError> {
    if let Some(id) = id {
        crate::domains::virtual_users::VIRTUAL_USER_MANAGE
            .evaluate_with(ctx.permission_resolver.as_ref(), &ctx.caller)
            .map_err(|_| {
                CommandError::forbidden(
                    "Service account binding requires virtual-user management permission",
                )
            })?;
        let user = ctx
            .db
            .get_virtual_user(ctx.org_id(), id)
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Virtual user"))?;
        if user.status != "active" || user.usage != "service" {
            return Err(CommandError::bad_request(
                "Agent requires an active service virtual user",
            ));
        }
    }
    Ok(())
}

pub(super) fn validate_create_limits(req: &CreateAgentRequest) -> Result<(), CommandError> {
    if req.name.len() > MAX_AGENT_NAME_BYTES
        || req
            .display_name
            .as_ref()
            .is_some_and(|d| d.len() > MAX_AGENT_NAME_BYTES)
        || req
            .description
            .as_ref()
            .is_some_and(|d| d.len() > MAX_AGENT_DESCRIPTION_BYTES)
        || req.system_prompt.len() > MAX_AGENT_SYSTEM_PROMPT_BYTES
        || req.capabilities.len() > MAX_AGENT_CAPABILITIES
        || req.initial_files.len() > MAX_INITIAL_FILES
        || initial_files_total_bytes(&req.initial_files) > MAX_INITIAL_FILES_TOTAL_BYTES
    {
        return Err(CommandError::bad_request("Input exceeds allowed limits"));
    }
    check_platform_chat_content(
        req.intro_markdown.as_deref(),
        req.short_description.as_deref(),
        &req.starters,
    )
    .map_err(CommandError::bad_request)?;
    Ok(())
}

pub(super) fn validate_update_limits(req: &UpdateAgentRequest) -> Result<(), CommandError> {
    if req
        .display_name
        .as_ref()
        .is_some_and(|d| d.len() > MAX_AGENT_NAME_BYTES)
        || req
            .description
            .as_ref()
            .is_some_and(|d| d.len() > MAX_AGENT_DESCRIPTION_BYTES)
        || req
            .system_prompt
            .as_ref()
            .is_some_and(|s| s.len() > MAX_AGENT_SYSTEM_PROMPT_BYTES)
        || req
            .capabilities
            .as_ref()
            .is_some_and(|c| c.len() > MAX_AGENT_CAPABILITIES)
        || req
            .initial_files
            .as_ref()
            .is_some_and(|f| f.len() > MAX_INITIAL_FILES)
        || req
            .initial_files
            .as_ref()
            .is_some_and(|f| initial_files_total_bytes(f) > MAX_INITIAL_FILES_TOTAL_BYTES)
    {
        return Err(CommandError::bad_request("Input exceeds allowed limits"));
    }
    check_platform_chat_content(
        req.intro_markdown.as_ref().and_then(|v| v.as_deref()),
        req.short_description.as_ref().and_then(|v| v.as_deref()),
        req.starters.as_deref().unwrap_or_default(),
    )
    .map_err(CommandError::bad_request)?;
    Ok(())
}

pub(super) fn initial_files_total_bytes(files: &[InitialFile]) -> usize {
    files.iter().map(|f| f.content.len()).sum()
}
