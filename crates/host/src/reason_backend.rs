//! Selects the provider loop for a Reason activity: the opt-in OpenAI Agents
//! API backend when the turn selected it and the host can serve it, the
//! native loop (with native async tools) otherwise.

pub(crate) async fn execute_reason<A: crate::RuntimeHostAdapter>(
    adapter: &A,
    org_id: i64,
    input: everruns_engine::ReasonInput,
    assembled: everruns_core::AssembledTurnContext,
    atom: everruns_engine::ReasonAtom,
) -> everruns_provider::error::Result<everruns_engine::ReasonResult> {
    #[cfg(feature = "openai-agents-api")]
    if let Some(result) =
        crate::openai_agents_api::backend::try_execute_reason(adapter, org_id, &input, &assembled)
            .await?
    {
        return Ok(result);
    }
    crate::native_async::execute_reason(adapter, org_id, input, assembled, atom).await
}
