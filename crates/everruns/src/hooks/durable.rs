//! Conservative recovery of awaited application effects. The invocation marker
//! commits before dispatch. Unknown effects never rerun and gates never open.
use super::*;
use crate::hosting::SessionEpoch;
use serde_json::json;

pub(super) async fn invoke<F, Fut>(
    epoch: Option<&Arc<dyn SessionEpoch>>,
    id: &str,
    policy: HookRecovery,
    effect: F,
) -> Result<(), String>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<(), String>>,
{
    let Some(epoch) = epoch else {
        return effect().await;
    };
    let key = format!("hook/{id}");
    let first = epoch
        .commit(&key, json!({"state":"invoked","policy":policy}), true)
        .await
        .map_err(|e| e.to_string())?;
    if !first {
        let record = epoch.record(&key).await.map_err(|e| e.to_string())?;
        if record.as_ref().and_then(|record| record.get("policy")) != Some(&json!(policy)) {
            return Err(
                "retained lifecycle recovery policy conflicts with registered behavior".into(),
            );
        }
        if let Some(result) = record.as_ref().and_then(|r| r.get("result")) {
            return serde_json::from_value(result.clone())
                .map_err(|_| "corrupt lifecycle receipt".to_string())?;
        }
        if policy == HookRecovery::AtMostOnce {
            return Err(
                "lifecycle effect has an uncertain outcome; automatic retry refused".into(),
            );
        }
    }
    let result = effect().await;
    epoch
        .commit(
            &key,
            json!({"state":"settled","policy":policy,"result":result}),
            false,
        )
        .await
        .map_err(|e| e.to_string())?;
    result
}

impl HookRunState {
    /// Tool receipts may outlive the process-local failure accumulator. Recover
    /// their failures (including an uncertain dispatch) before finalization.
    pub(crate) async fn recovered_tool_failures(
        &self,
        turn: &str,
    ) -> Result<Vec<HookFailure>, everruns_contracts::error::AgentLoopError> {
        let Some(epoch) = &self.epoch else {
            return Ok(Vec::new());
        };
        let prefix = format!("hook/{turn}/");
        let mut failures = Vec::new();
        for (key, record) in epoch.records(&prefix).await? {
            let Some((point, call)) = key
                .strip_prefix(&prefix)
                .and_then(|key| key.split_once('/'))
            else {
                continue;
            };
            let point = match point {
                "tool_start" => HookPoint::ToolStart,
                "tool_end" => HookPoint::ToolEnd,
                _ => continue,
            };
            let Some((call, index)) = call.rsplit_once('/') else {
                continue;
            };
            let Ok(index) = index.parse() else {
                continue;
            };
            let message = match record.get("result") {
                Some(result) => {
                    match serde_json::from_value::<Result<(), String>>(result.clone()) {
                        Ok(Err(message)) => Some(message),
                        Ok(Ok(())) => None,
                        Err(_) => Some("invalid lifecycle outcome receipt".into()),
                    }
                }
                None => {
                    Some("lifecycle effect has an uncertain outcome after executor loss".into())
                }
            };
            if let Some(message) = message {
                failures.push(HookFailure {
                    point,
                    handler_index: index,
                    message,
                    tool_name: None,
                    tool_call_id: Some(call.into()),
                });
            }
        }
        Ok(failures)
    }
    pub(super) async fn record_durable_failure(&self, id: &str, failure: &HookFailure) {
        if let Some(epoch) = &self.epoch {
            if let Err(error) = epoch
                .commit(&format!("failure/{id}"), json!(failure), true)
                .await
            {
                tracing::warn!(%error, "Lifecycle failure receipt could not be committed");
            }
        }
    }
    pub(crate) fn durable(hooks: LifecycleHooks, epoch: Arc<dyn SessionEpoch>) -> Arc<Self> {
        Arc::new(Self {
            hooks,
            failures: Mutex::new(Vec::new()),
            epoch: Some(epoch),
        })
    }

    pub(crate) async fn durable_agent_start(
        &self,
        input: crate::MessageId,
        context: AgentStartContext,
    ) -> Result<(), HookFailure> {
        for (index, handler) in self.hooks.agent_start.iter().enumerate() {
            let completed = format!("hook/agent_start/{index}");
            if let Some(epoch) = &self.epoch {
                let record = epoch.record(&completed).await.map_err(|error| {
                    HookFailure::new(HookPoint::AgentStart, index, error.to_string())
                })?;
                if let Some(record) = record {
                    if record.get("policy") != Some(&json!(handler.recovery)) {
                        return Err(HookFailure::new(
                            HookPoint::AgentStart,
                            index,
                            "retained lifecycle recovery policy conflicts with registered behavior"
                                .into(),
                        ));
                    }
                    if record.get("result") == Some(&json!(Ok::<(), String>(()))) {
                        continue;
                    }
                }
                // Cancellation can drop preparation after its attempt receipt
                // settled but before the session-wide success marker committed.
                // Recover that boundary before considering a new input's effect.
                let attempts = epoch
                    .records(&format!("hook/agent_start/{index}/attempt/"))
                    .await
                    .map_err(|error| {
                        HookFailure::new(HookPoint::AgentStart, index, error.to_string())
                    })?;
                let mut settled = false;
                for (_, record) in attempts {
                    if record.get("policy") != Some(&json!(handler.recovery)) {
                        return Err(HookFailure::new(
                            HookPoint::AgentStart,
                            index,
                            "retained lifecycle recovery policy conflicts with registered behavior"
                                .into(),
                        ));
                    }
                    match record.get("result").and_then(|result|serde_json::from_value::<Result<(),String>>(result.clone()).ok()) {
                        Some(Ok(()))=>settled=true,
                        Some(Err(_))=>{},
                        None if handler.recovery==HookRecovery::AtMostOnce=>return Err(HookFailure::new(HookPoint::AgentStart,index,"lifecycle effect has an uncertain outcome; automatic retry refused".into())),
                        None=>{},
                    }
                }
                if settled {
                    epoch.commit(&completed,json!({"state":"settled","policy":handler.recovery,"result":Ok::<(),String>(())}),false).await.map_err(|error|HookFailure::new(HookPoint::AgentStart,index,error.to_string()))?;
                    continue;
                }
            }
            invoke(
                self.epoch.as_ref(),
                &format!("agent_start/{index}/attempt/{input}"),
                handler.recovery,
                || handler(context.clone()),
            )
            .await
            .map_err(|message| HookFailure::new(HookPoint::AgentStart, index, message))?;
            if let Some(epoch) = &self.epoch {
                epoch.commit(&completed,json!({"state":"settled","policy":handler.recovery,"result":Ok::<(),String>(())}),false).await.map_err(|error|HookFailure::new(HookPoint::AgentStart,index,error.to_string()))?;
            }
        }
        Ok(())
    }

    // Only a matching, settled handler error may reject this input and permit
    // a later explicit command. Missing receipts and policy drift suspend.
    pub(crate) async fn gate_failure_is_known(
        &self,
        input: crate::MessageId,
        turn: &str,
        failure: &HookFailure,
    ) -> everruns_contracts::error::Result<bool> {
        let (key, policy) = match failure.point {
            HookPoint::AgentStart => (
                format!("hook/agent_start/{}/attempt/{input}", failure.handler_index),
                self.hooks
                    .agent_start
                    .get(failure.handler_index)
                    .map(|handler| handler.recovery),
            ),
            HookPoint::TurnStart => (
                format!("hook/{turn}/turn_start/{}", failure.handler_index),
                self.hooks
                    .turn_start
                    .get(failure.handler_index)
                    .map(|handler| handler.recovery),
            ),
            _ => return Ok(false),
        };
        let Some(epoch) = &self.epoch else {
            return Ok(false);
        };
        let Some(record) = epoch.record(&key).await? else {
            return Ok(false);
        };
        if policy.map(|policy| json!(policy)).as_ref() != record.get("policy") {
            return Ok(false);
        }
        Ok(record
            .get("result")
            .and_then(|value| serde_json::from_value::<Result<(), String>>(value.clone()).ok())
            .is_some_and(|result| matches!(result,Err(message) if message==failure.message)))
    }

    pub(crate) async fn durable_turn_start(
        &self,
        turn: &str,
        context: TurnStartContext,
    ) -> Result<(), HookFailure> {
        for (index, handler) in self.hooks.turn_start.iter().enumerate() {
            invoke(
                self.epoch.as_ref(),
                &format!("{turn}/turn_start/{index}"),
                handler.recovery,
                || handler(context.clone()),
            )
            .await
            .map_err(|message| HookFailure::new(HookPoint::TurnStart, index, message))?;
        }
        Ok(())
    }

    pub(crate) async fn durable_completion(&self, context: CompletionContext) -> Vec<HookFailure> {
        let mut failures = Vec::new();
        for (index, handler) in self.hooks.completion.iter().enumerate() {
            if let Err(message) = invoke(
                self.epoch.as_ref(),
                &format!("{}/completion/{index}", context.turn.turn_id),
                handler.recovery,
                || handler(context.clone()),
            )
            .await
            {
                failures.push(HookFailure::new(HookPoint::Completion, index, message));
            }
        }
        failures
    }
}
