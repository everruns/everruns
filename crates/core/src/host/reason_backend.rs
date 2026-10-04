//! Selects the provider loop for a Reason activity: the opt-in OpenAI Agents
//! API backend when the turn selected it and the host can serve it, the
//! native loop (with native async tools) otherwise.

pub(crate) async fn execute_reason<A: crate::host::RuntimeHostAdapter>(
    adapter: &A,
    org_id: i64,
    input: crate::engine::ReasonInput,
    assembled: crate::AssembledTurnContext,
    atom: crate::engine::ReasonAtom,
) -> everruns_contracts::error::Result<crate::engine::ReasonResult> {
    #[cfg(feature = "openai-agents-api")]
    if let Some(result) = crate::host::openai_agents_api::backend::try_execute_reason(
        adapter, org_id, &input, &assembled,
    )
    .await?
    {
        return Ok(result);
    }
    // Background provider calls re-attach after a restart and stop on an
    // explicit turn cancel (EVE-1134).
    let atom = atom.with_background_call(crate::host::background_call::context(
        adapter,
        org_id,
        input.context.session_id,
        input.context.turn_id,
    ));
    crate::host::native_async::execute_reason(adapter, org_id, input, assembled, atom).await
}
