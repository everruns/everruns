// The `user_mcp` store and prompter for hosts that run the calls elsewhere.
//
// The hosted worker cannot reach the person's list itself: the control plane
// owns it and resolves the person from the turn's input message. These adapters
// turn each trait call into one `UserMcpStoreCall` for an invoker the host
// supplies (in-process, or over gRPC), bound to that input message.

use async_trait::async_trait;
use everruns_core::ScopedMcpServer;
use everruns_core::mcp::{
    McpLogin, McpLoginPrompter, UserMcpServerEntry, UserMcpServerSummary, UserMcpStore,
    UserMcpStoreCall, UserMcpStoreError, UserMcpStoreReply, UserMcpStoreResult,
};
use everruns_core::tool_context::ToolContextExtensions;
use std::sync::Arc;
use uuid::Uuid;

use super::{McpLoginPrompterExt, UserMcpStoreExt};

/// Runs one store or prompter call for a session, as the person who sent
/// `input_message`. Bound by the host to one org and session.
#[async_trait]
pub trait UserMcpCallInvoker: Send + Sync {
    async fn invoke(
        &self,
        input_message: Option<Uuid>,
        call: UserMcpStoreCall,
    ) -> UserMcpStoreResult<UserMcpStoreReply>;
}

/// [`UserMcpStore`] and [`McpLoginPrompter`] over a [`UserMcpCallInvoker`].
#[derive(Clone)]
pub struct ForwardingUserMcpStore {
    invoker: Arc<dyn UserMcpCallInvoker>,
    input_message: Option<Uuid>,
}

impl ForwardingUserMcpStore {
    pub fn new(invoker: Arc<dyn UserMcpCallInvoker>, input_message: Option<Uuid>) -> Self {
        Self {
            invoker,
            input_message,
        }
    }

    async fn call(&self, call: UserMcpStoreCall) -> UserMcpStoreResult<UserMcpStoreReply> {
        self.invoker.invoke(self.input_message, call).await
    }
}

fn unexpected(reply: UserMcpStoreReply) -> UserMcpStoreError {
    UserMcpStoreError::Internal(format!("unexpected user MCP store reply: {reply:?}"))
}

#[async_trait]
impl UserMcpStore for ForwardingUserMcpStore {
    async fn list(&self) -> UserMcpStoreResult<Vec<UserMcpServerSummary>> {
        match self.call(UserMcpStoreCall::List).await? {
            UserMcpStoreReply::Servers { servers } => Ok(servers),
            other => Err(unexpected(other)),
        }
    }

    async fn upsert(
        &self,
        name: &str,
        entry: UserMcpServerEntry,
    ) -> UserMcpStoreResult<UserMcpServerSummary> {
        let call = UserMcpStoreCall::Upsert {
            name: name.to_string(),
            entry: Box::new(entry),
        };
        match self.call(call).await? {
            UserMcpStoreReply::Server { server } => Ok(server),
            other => Err(unexpected(other)),
        }
    }

    async fn remove(&self, name: &str) -> UserMcpStoreResult<bool> {
        let call = UserMcpStoreCall::Remove {
            name: name.to_string(),
        };
        match self.call(call).await? {
            UserMcpStoreReply::Removed { removed } => Ok(removed),
            other => Err(unexpected(other)),
        }
    }

    async fn add_to_chat(
        &self,
        name: &str,
        server: ScopedMcpServer,
    ) -> UserMcpStoreResult<UserMcpServerSummary> {
        let call = UserMcpStoreCall::AddToChat {
            name: name.to_string(),
            server: Box::new(server),
        };
        match self.call(call).await? {
            UserMcpStoreReply::Server { server } => Ok(server),
            other => Err(unexpected(other)),
        }
    }

    async fn set_enabled(
        &self,
        name: &str,
        enabled: bool,
    ) -> UserMcpStoreResult<UserMcpServerSummary> {
        let call = UserMcpStoreCall::SetEnabled {
            name: name.to_string(),
            enabled,
        };
        match self.call(call).await? {
            UserMcpStoreReply::Server { server } => Ok(server),
            other => Err(unexpected(other)),
        }
    }
}

#[async_trait]
impl McpLoginPrompter for ForwardingUserMcpStore {
    async fn start_login(&self, name: &str) -> UserMcpStoreResult<McpLogin> {
        let call = UserMcpStoreCall::StartLogin {
            name: name.to_string(),
        };
        match self.call(call).await? {
            UserMcpStoreReply::Login { login } => Ok(login),
            other => Err(unexpected(other)),
        }
    }
}

/// Install the store and prompter on a tool context, bound to `input_message`
/// (None until the act binds the turn's input, which the host then refuses).
pub fn install_user_mcp_store(
    extensions: &mut ToolContextExtensions,
    invoker: Arc<dyn UserMcpCallInvoker>,
    input_message: Option<Uuid>,
) {
    let store = Arc::new(ForwardingUserMcpStore::new(invoker, input_message));
    extensions.insert(Arc::new(UserMcpStoreExt(store.clone())));
    extensions.insert(Arc::new(McpLoginPrompterExt(store)));
}
