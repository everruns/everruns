//! `SessionSpendReader` over the `worker_get_session_spend` command.
//!
//! A worker or a data plane executor has no cost tables, so the session
//! asks the control plane what it spent (`get_session_info`). The command
//! reads only the asking session and its subagents.

use super::{InternalCommandTransport, call};
use async_trait::async_trait;
use everruns_contracts::error::Result;
use everruns_contracts::runtime::session_spend::{SessionSpend, SessionSpendReader};
use everruns_contracts::typed_id::{SessionId, TurnId};

const COMMAND: &str = "worker_get_session_spend";

/// The session spend reader a worker or an executor installs.
pub struct CommandSessionSpendReader<T> {
    transport: T,
}

impl<T: InternalCommandTransport> CommandSessionSpendReader<T> {
    pub fn new(transport: T) -> Self {
        Self { transport }
    }
}

#[async_trait]
impl<T: InternalCommandTransport + 'static> SessionSpendReader for CommandSessionSpendReader<T> {
    async fn session_spend(
        &self,
        session: SessionId,
        turn: Option<TurnId>,
    ) -> Result<Option<SessionSpend>> {
        let params = serde_json::json!({
            "session_id": session.to_string(),
            "turn_id": turn.map(|t| t.to_string()),
        });
        call(&self.transport, "Session spend", COMMAND, params)
            .await
            .map(Some)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proto;
    use serde_json::Value;
    use std::sync::Mutex;

    struct Recorded(Mutex<Vec<(String, Value)>>, Value);

    #[async_trait]
    impl InternalCommandTransport for Recorded {
        async fn execute_internal_command(
            &self,
            name: &str,
            params: Value,
        ) -> Result<std::result::Result<Value, proto::CommandError>> {
            self.0.lock().unwrap().push((name.to_owned(), params));
            Ok(Ok(self.1.clone()))
        }
    }

    #[tokio::test]
    async fn asks_for_its_own_session_and_turn() {
        let spend = everruns_contracts::runtime::session_spend::summarize(
            "s".into(),
            &Default::default(),
            vec![],
            Default::default(),
            None,
        );
        let reader = CommandSessionSpendReader::new(Recorded(
            Mutex::new(Vec::new()),
            serde_json::to_value(&spend).unwrap(),
        ));
        let session = SessionId::new();
        let turn = TurnId::new();
        let got = reader.session_spend(session, Some(turn)).await.unwrap();
        assert_eq!(got, Some(spend));
        let calls = reader.transport.0.lock().unwrap();
        assert_eq!(calls[0].0, COMMAND);
        assert_eq!(calls[0].1["session_id"], session.to_string());
        assert_eq!(calls[0].1["turn_id"], turn.to_string());
    }
}
