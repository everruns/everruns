use super::*;

pub(super) async fn authorize_session(
    state: &AppState,
    org: &ResolvedOrg,
    session_id: SessionId,
) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    // THREAT[TM-TENANT-018]: the lookup is org-scoped, so another org's session
    // id comes back as `Ok(None)`, and that must reject. Every voice route then
    // writes leased resources and events keyed by the caller-supplied id, and
    // those writes are not org-scoped. Foreign, missing, and failed lookups all
    // answer the same 404 so a foreign session's existence is not disclosed.
    let session = state
        .session_service
        .get(&Caller::from(org), session_id.uuid(), None)
        .await
        .map_err(|err| {
            tracing::debug!(error = %err, "voice session authorization failed");
            ErrorResponse::not_found("Session")
        })?
        .ok_or_else(|| ErrorResponse::not_found("Session"))?;
    // Realtime transcripts bypass runtime invocation binding. Until that path
    // supports the fixed test subject, Playground uses the shared text composer.
    if session.source == everruns_platform::SessionSource::Playground {
        return Err(ErrorResponse::new("Voice is unavailable in Playground")
            .into_response(StatusCode::BAD_REQUEST));
    }
    Ok(())
}
