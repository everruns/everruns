//! Native asynchronous calls are pending provider exchanges, not background-run handles.
//!
//! Checkpoints must be saved before dispatch and before sending a continuation.
//! The host must fence concurrent owners; a replay never retries an ambiguous
//! provider delivery. This prevents duplicate outputs and lost pending work.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;

use crate::error::{AgentLoopError, Result};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type")]
pub enum NativeToolCall {
    #[serde(rename = "function_call")]
    Function {
        call_id: String,
        name: String,
        arguments: String,
        #[serde(rename = "async", default)]
        asynchronous: bool,
    },
    #[serde(rename = "custom_tool_call")]
    Custom {
        call_id: String,
        name: String,
        input: String,
        #[serde(rename = "async", default)]
        asynchronous: bool,
    },
}

impl NativeToolCall {
    pub fn id(&self) -> &str {
        match self {
            Self::Function { call_id, .. } | Self::Custom { call_id, .. } => call_id,
        }
    }
    pub fn name(&self) -> &str {
        match self {
            Self::Function { name, .. } | Self::Custom { name, .. } => name,
        }
    }
    pub fn is_async(&self) -> bool {
        match self {
            Self::Function { asynchronous, .. } | Self::Custom { asynchronous, .. } => {
                *asynchronous
            }
        }
    }
    pub fn output(&self, output: &str) -> Value {
        json!({"type": match self { Self::Function { .. } => "function_call_output", Self::Custom { .. } => "custom_tool_call_output" }, "call_id": self.id(), "output": output})
    }
    pub fn validate(&self) -> Result<()> {
        if self.id().is_empty() || self.name().is_empty() {
            return Err(AgentLoopError::llm(
                "native tool call is missing its call_id or name",
            ));
        }
        if let Self::Function { arguments, .. } = self {
            let _: Value = serde_json::from_str(arguments).map_err(|_| {
                AgentLoopError::llm("completed function call has invalid JSON arguments")
            })?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum PendingCallState {
    Queued,
    Running,
    Ready { output: String },
    Delivered,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PendingCall {
    pub call: NativeToolCall,
    /// Only the trusted executor may grant replay safety; never infer it from model input.
    pub replay_safe: bool,
    pub state: PendingCallState,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Delivery {
    pub previous_response_id: String,
    pub call_ids: Vec<String>,
    pub input: Vec<Value>,
}

/// Serializable state for one conversation. Delivered IDs remain as tombstones.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct NativeAsyncCheckpoint {
    pub latest_response_id: Option<String>,
    #[serde(default)]
    pub response_in_flight: bool,
    pub calls: BTreeMap<String, PendingCall>,
    /// Arrival order is retained for recovery of conflicting synchronous jobs.
    pub order: Vec<String>,
    /// Persist before HTTP submission. Recovery requires an explicit receipt if
    /// the process died between the provider accepting outputs and saving its ID.
    pub delivery: Option<Delivery>,
}

impl NativeAsyncCheckpoint {
    /// Register only after authorization, before launching work. Repeated stream
    /// items with the same payload are harmless; reused IDs with new inputs fail.
    pub fn register(&mut self, call: NativeToolCall, replay_safe: bool) -> Result<bool> {
        call.validate()?;
        if let Some(existing) = self.calls.get(call.id()) {
            if existing.call != call || existing.replay_safe != replay_safe {
                return Err(AgentLoopError::config(
                    "native call_id reused with different inputs or policy",
                ));
            }
            return Ok(false);
        }
        self.order.push(call.id().to_owned());
        self.calls.insert(
            call.id().to_owned(),
            PendingCall {
                call,
                replay_safe,
                state: PendingCallState::Queued,
            },
        );
        Ok(true)
    }

    pub fn start(&mut self, id: &str) -> Result<()> {
        let pending = self
            .calls
            .get_mut(id)
            .ok_or_else(|| AgentLoopError::config("unknown native call_id"))?;
        if pending.state != PendingCallState::Queued {
            return Err(AgentLoopError::config("native call is not queued"));
        }
        pending.state = PendingCallState::Running;
        Ok(())
    }

    /// First result wins. A late result after cancellation cannot overwrite it.
    pub fn settle(&mut self, id: &str, output: String) -> Result<bool> {
        let pending = self
            .calls
            .get_mut(id)
            .ok_or_else(|| AgentLoopError::config("unknown native call_id"))?;
        if matches!(
            pending.state,
            PendingCallState::Ready { .. } | PendingCallState::Delivered
        ) {
            return Ok(false);
        }
        pending.state = PendingCallState::Ready { output };
        Ok(true)
    }

    pub fn response_completed(&mut self, response_id: String) -> Result<()> {
        if response_id.is_empty() || self.delivery.is_some() {
            return Err(AgentLoopError::config(
                "response ID missing or delivery requires acknowledgement",
            ));
        }
        self.latest_response_id = Some(response_id);
        Ok(())
    }

    /// Recovery runs only after the previous execution owner has been fenced.
    pub fn recover(&mut self) -> Result<()> {
        let ordered: std::collections::BTreeSet<_> = self.order.iter().collect();
        if ordered.len() != self.calls.len()
            || self.order.len() != self.calls.len()
            || self
                .calls
                .iter()
                .any(|(id, pending)| id != pending.call.id() || !ordered.contains(id))
        {
            return Err(AgentLoopError::store(
                "native checkpoint call registry is inconsistent",
            ));
        }
        for pending in self.calls.values() {
            pending.call.validate()?;
        }

        if self.delivery.is_some() || self.response_in_flight {
            return Err(AgentLoopError::store(
                "native result delivery is uncertain; reconcile the response receipt before resuming",
            ));
        }
        for pending in self.calls.values_mut() {
            if pending.state == PendingCallState::Running {
                pending.state = if pending.replay_safe {
                    PendingCallState::Queued
                } else {
                    PendingCallState::Ready { output: json!({"error":"interrupted; execution outcome is uncertain; do not retry automatically"}).to_string() }
                };
            }
        }
        Ok(())
    }

    pub fn cancel(&mut self) {
        for pending in self.calls.values_mut() {
            if matches!(
                pending.state,
                PendingCallState::Queued | PendingCallState::Running
            ) {
                pending.state = PendingCallState::Ready {
                    output: json!({"error":"cancelled"}).to_string(),
                };
            }
        }
    }

    /// Newly available results are sent on their original call IDs using the
    /// latest response, even when independent responses intervened. A host may
    /// append a synchronous wait status *after* this input, never before it.
    pub fn prepare_delivery(&mut self) -> Result<Option<&Delivery>> {
        if self.response_in_flight {
            return Err(AgentLoopError::store(
                "cannot deliver outputs while the latest response is incomplete",
            ));
        }
        if self.delivery.is_some() {
            return Err(AgentLoopError::store("native delivery already in flight"));
        }
        let ready: Vec<_> = self
            .calls
            .iter()
            .filter_map(|(id, pending)| {
                if let PendingCallState::Ready { output } = &pending.state {
                    Some((id.clone(), pending.call.output(output)))
                } else {
                    None
                }
            })
            .collect();
        if ready.is_empty() {
            return Ok(None);
        }
        let previous_response_id = self.latest_response_id.clone().ok_or_else(|| {
            AgentLoopError::store("cannot deliver native outputs before a response completes")
        })?;
        let (call_ids, input) = ready.into_iter().unzip();
        self.delivery = Some(Delivery {
            previous_response_id,
            call_ids,
            input,
        });
        Ok(self.delivery.as_ref())
    }

    /// Acknowledge only a successfully completed provider response (or a receipt
    /// recovered by the host). A duplicate receipt is harmless.
    pub fn acknowledge_delivery(&mut self, response_id: String) -> Result<()> {
        if response_id.is_empty() {
            return Err(AgentLoopError::config("empty native delivery response ID"));
        }
        if self
            .delivery
            .as_ref()
            .is_some_and(|delivery| delivery.previous_response_id == response_id)
        {
            return Err(AgentLoopError::store(
                "native delivery receipt must identify a new response",
            ));
        }
        let Some(delivery) = self.delivery.take() else {
            if self.latest_response_id.as_ref() == Some(&response_id) {
                return Ok(());
            }
            return Err(AgentLoopError::store(
                "no native delivery awaiting acknowledgement",
            ));
        };
        for id in delivery.call_ids {
            self.calls
                .get_mut(&id)
                .expect("delivery references registered calls")
                .state = PendingCallState::Delivered;
        }
        self.latest_response_id = Some(response_id);
        Ok(())
    }

    pub fn can_complete(&self) -> bool {
        !self.response_in_flight
            && self.delivery.is_none()
            && self
                .calls
                .values()
                .all(|pending| pending.state == PendingCallState::Delivered)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn call(id: &str) -> NativeToolCall {
        NativeToolCall::Function {
            call_id: id.into(),
            name: "lookup".into(),
            arguments: "{}".into(),
            asynchronous: true,
        }
    }
    #[test]
    fn outputs_follow_latest_response_and_deduplicate() {
        let mut state = NativeAsyncCheckpoint::default();
        state.register(call("slow"), true).unwrap();
        state.register(call("fast"), true).unwrap();
        assert!(!state.register(call("fast"), true).unwrap());
        state.start("slow").unwrap();
        state.start("fast").unwrap();
        state.response_completed("response_launch".into()).unwrap();
        state
            .response_completed("response_independent".into())
            .unwrap();
        state.settle("fast", "first".into()).unwrap();
        assert!(!state.settle("fast", "duplicate".into()).unwrap());
        let delivery = state.prepare_delivery().unwrap().unwrap();
        assert_eq!(delivery.previous_response_id, "response_independent");
        assert_eq!(
            delivery.input,
            vec![json!({"type":"function_call_output","call_id":"fast","output":"first"})]
        );
        assert!(!state.can_complete());
        state.acknowledge_delivery("response_fast".into()).unwrap();
        state.acknowledge_delivery("response_fast".into()).unwrap();
        state.settle("slow", "last".into()).unwrap();
        assert_eq!(
            state
                .prepare_delivery()
                .unwrap()
                .unwrap()
                .previous_response_id,
            "response_fast"
        );
        state.acknowledge_delivery("response_final".into()).unwrap();
        assert!(state.can_complete());
        assert!(!state.register(call("fast"), true).unwrap());
    }
    #[test]
    fn recovery_replays_only_safe_work_and_preserves_cancellation() {
        let mut state = NativeAsyncCheckpoint::default();
        state.register(call("safe"), true).unwrap();
        state.register(call("unsafe"), false).unwrap();
        state.start("safe").unwrap();
        state.start("unsafe").unwrap();
        let mut recovered: NativeAsyncCheckpoint =
            serde_json::from_str(&serde_json::to_string(&state).unwrap()).unwrap();
        recovered.recover().unwrap();
        assert_eq!(recovered.calls["safe"].state, PendingCallState::Queued);
        assert!(matches!(
            recovered.calls["unsafe"].state,
            PendingCallState::Ready { .. }
        ));
        recovered.cancel();
        assert!(!recovered.settle("safe", "too late".into()).unwrap());
        assert!(!recovered.can_complete());
        recovered.response_completed("r1".into()).unwrap();
        recovered.prepare_delivery().unwrap();
        let before = recovered.clone();
        assert!(recovered.recover().is_err());
        assert_eq!(recovered, before);
        recovered
            .acknowledge_delivery("reconciled_receipt".into())
            .unwrap();
        assert!(recovered.can_complete());
    }
    #[test]
    fn custom_calls_retain_input_async_and_original_id() {
        let call: NativeToolCall = serde_json::from_value(json!({"type":"custom_tool_call","call_id":"custom","name":"query","input":"a raw query","async":true})).unwrap();
        assert!(call.is_async());
        assert_eq!(call.output("result")["type"], "custom_tool_call_output");
        assert_eq!(serde_json::to_value(&call).unwrap()["async"], true);
        assert_eq!(call.output("result")["call_id"], "custom");
    }
    #[test]
    fn reused_ids_and_invalid_calls_fail_closed() {
        let mut state = NativeAsyncCheckpoint::default();
        state.register(call("same"), true).unwrap();
        let changed = NativeToolCall::Function {
            call_id: "same".into(),
            name: "lookup".into(),
            arguments: "{\"different\":true}".into(),
            asynchronous: true,
        };
        assert!(state.register(changed, true).is_err());
        assert!(state.register(call(""), true).is_err());
        let malformed = NativeToolCall::Function {
            call_id: "invalid".into(),
            name: "lookup".into(),
            arguments: "{".into(),
            asynchronous: true,
        };
        assert!(state.register(malformed, true).is_err());
        state.response_in_flight = true;
        assert!(state.recover().is_err());
        assert!(!state.can_complete());
    }
}
