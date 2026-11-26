//! Private code execution state follows the session through Raft migration.
use super::*;
use everruns_contracts::runtime::durability::{
    DurableToolCallStatus, DurableToolResultStore, ToolCallClaimResult,
};

// Delimit components unambiguously; the store itself is scoped to one session.
fn key(namespace: &str, call: &str) -> String {
    format!("{}:{namespace}{call}", namespace.len())
}

#[async_trait]
impl DurableToolResultStore for ShardRecords {
    async fn try_claim_tool_call(
        &self,
        namespace: &str,
        call: &str,
        _name: &str,
        fingerprint: &str,
    ) -> Result<ToolCallClaimResult> {
        parsed(
            self.propose(RecordOp::CodeClaim {
                key: key(namespace, call),
                fingerprint: fingerprint.into(),
                token: uuid::Uuid::new_v4(),
            })
            .await?,
        )
    }
    async fn settle_tool_call(
        &self,
        namespace: &str,
        call: &str,
        value: Value,
        status: &str,
        token: uuid::Uuid,
    ) -> Result<bool> {
        if status != "settled" {
            return Err(AgentLoopError::config(
                "code journal supports immutable settlements only",
            ));
        }
        parsed(
            self.propose(RecordOp::CodeSettle {
                key: key(namespace, call),
                value,
                token,
            })
            .await?,
        )
    }
    async fn get_tool_call_status(
        &self,
        namespace: &str,
        call: &str,
    ) -> Result<Option<DurableToolCallStatus>> {
        let key = key(namespace, call);
        self.read(|records| {
            records
                .code_execution
                .get(&key)
                .map(|record| match &record.value {
                    Some(result_json) => DurableToolCallStatus::Settled {
                        result_json: result_json.clone(),
                    },
                    None => DurableToolCallStatus::Running,
                })
        })
        .await
    }
}
