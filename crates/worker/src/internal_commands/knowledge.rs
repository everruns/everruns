//! One knowledge runtime client shared by direct and gRPC workers.
use super::{InternalCommandTransport, call};
use async_trait::async_trait;
use everruns_contracts::{
    error::Result,
    knowledge::*,
    typed_id::{MessageId, SessionId},
};
use serde_json::json;
pub struct CommandKnowledgeRuntime<T> {
    transport: T,
}
impl<T> CommandKnowledgeRuntime<T> {
    pub fn new(transport: T) -> Self {
        Self { transport }
    }
}
#[async_trait]
impl<T: InternalCommandTransport> KnowledgeRuntime for CommandKnowledgeRuntime<T> {
    async fn search(&self, session: SessionId, query: KnowledgeQuery) -> Result<KnowledgeResults> {
        call(
            &self.transport,
            "Search Knowledge",
            "worker_knowledge",
            json!({"session_id":session,"operation":"search","query":query}),
        )
        .await
    }
    async fn files(
        &self,
        session: SessionId,
        query: KnowledgeCatalogQuery,
    ) -> Result<KnowledgeCatalog> {
        call(
            &self.transport,
            "Browse Knowledge",
            "worker_knowledge",
            json!({"session_id":session,"operation":"files","catalog":query}),
        )
        .await
    }
    async fn automatic_context(
        &self,
        session: SessionId,
        input: MessageId,
        query: String,
    ) -> Result<Option<String>> {
        call(&self.transport,"Retrieve Knowledge context","worker_knowledge",json!({"session_id":session,"operation":"automatic","input_id":input,"query":{"query":query}})).await
    }
}
