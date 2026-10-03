use super::*;

pub(super) async fn authorize_session(
    state: &AppState,
    org: &ResolvedOrg,
    session_id: SessionId,
) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
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
