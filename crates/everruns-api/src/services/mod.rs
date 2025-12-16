// Services layer for business logic (M2)
// Services own business logic and validation, calling storage directly

pub mod event;
pub mod harness;
pub mod session;

pub use event::EventService;
pub use harness::HarnessService;
pub use session::SessionService;
