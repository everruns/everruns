// Services layer for business logic (M2)
// Services own business logic and validation, calling storage directly

pub mod agent;
pub mod capability;
pub mod event;
pub mod llm_model;
pub mod llm_provider;
pub mod llm_resolver;
pub mod message;
pub mod session;
pub mod session_file;

pub use agent::AgentService;
pub use capability::CapabilityService;
pub use event::EventService;
pub use llm_model::LlmModelService;
pub use llm_provider::LlmProviderService;
pub use llm_resolver::{LlmResolverService, ResolvedModel};
pub use message::MessageService;
pub use session::SessionService;
pub use session_file::SessionFileService;
