//! Deterministic simulation and demo fixtures for testing
//! [Everruns](https://everruns.com) agents.
//!
//! `everruns-test-support` carries the pieces that used to live inside
//! `everruns-core` purely for tests, examples, and demos, so production
//! builds no longer ship them:
//!
//! - [`llmsim_driver`] — the in-process `llmsim` chat driver: fixed, echo,
//!   sequence, and scripted multi-turn responses, latency and error
//!   injection, effort/message capture (`sim` feature)
//! - [`in_memory_loop`] — a full in-memory agentic loop with no database or
//!   network (`sim` + `host` features)
//! - [`in_memory`] — deterministic message and event fixtures for isolated
//!   tests; hosted loops use canonical event history instead
//! - [`doubles`] — mock/echo/failing tool executors and a mock chat driver
//! - [`capabilities`] — fake AWS/CRM/financial/warehouse demo capabilities
//!   and the test math/weather, sample-data, and noop fixtures
//!   (`fixtures` feature)
//! - [`LlmSimRuntimeExt`] — registration-only `.llm_sim(...)` and explicit
//!   `.llm_sim_as_default(...)` sugar for
//!   `everruns_host::InProcessRuntimeBuilder` (`host` feature)
//!
//! Production compositions must not register the fixture capabilities;
//! ordinary applications use `everruns::Model::simulated` instead of
//! depending on this crate directly.
//!
//! # Example
//!
//! ```
//! # #[tokio::main]
//! # async fn main() -> Result<(), Box<dyn std::error::Error>> {
//! use everruns_test_support::{InMemoryAgenticLoop, TestMathCapability};
//!
//! let agent = InMemoryAgenticLoop::builder()
//!     .with_simulated_response("2 + 2 = 4")
//!     .capability(TestMathCapability)
//!     .build()
//!     .await?;
//! let result = agent.run_turn("What is 2 + 2?").await?;
//! assert!(result.success);
//! # Ok(())
//! # }
//! ```

// LLM Simulator driver (behind the `sim` feature).
#[cfg(feature = "sim")]
pub mod llmsim_driver;

// In-memory agentic loop built on the llmsim driver.
#[cfg(all(feature = "sim", feature = "host"))]
pub mod in_memory_loop;

pub mod in_memory;

// Test doubles for the core execution traits.
pub mod doubles;

// Demo/test fixture capabilities (behind the `fixtures` feature).
#[cfg(feature = "fixtures")]
pub mod capabilities;

// Simulator extensions for the in-process host runtime builder.
#[cfg(feature = "host")]
mod runtime_ext;

#[cfg(feature = "host")]
mod store_reason_atom;

pub use in_memory::{InMemoryEventEmitter, InMemoryMessageRetriever};
#[cfg(all(feature = "sim", feature = "host"))]
pub use in_memory_loop::{InMemoryAgenticLoop, InMemoryAgenticLoopBuilder};
#[cfg(feature = "sim")]
pub use llmsim_driver::{
    LlmSimConfig, LlmSimDriver, OnExhausted, ResponseConfig, SimError, SimToolCall, SimTurn,
    ToolCallConfig, ToolCallPattern,
};

pub use doubles::{
    EchoToolExecutor, FailingToolExecutor, MockLlmResponse, MockProvider, MockToolExecutor,
};

#[cfg(feature = "fixtures")]
pub use capabilities::{
    FAKE_AWS_CAPABILITY_ID, FAKE_CRM_CAPABILITY_ID, FAKE_FINANCIAL_CAPABILITY_ID,
    FAKE_WAREHOUSE_CAPABILITY_ID, FakeAwsCapability, FakeCrmCapability, FakeFinancialCapability,
    FakeWarehouseCapability, NOOP_CAPABILITY_ID, NoopCapability, SAMPLE_DATA_CAPABILITY_ID,
    SampleDataCapability, TEST_MATH_CAPABILITY_ID, TEST_WEATHER_CAPABILITY_ID, TestMathCapability,
    TestWeatherCapability,
};

#[cfg(feature = "host")]
pub use runtime_ext::{LLMSIM_MODEL_ID, LLMSIM_PROVIDER, LlmSimRuntimeExt, llm_sim_provider};
#[cfg(feature = "host")]
pub use store_reason_atom::reason_atom_with_stores;
