//! In-memory loops, test doubles, writable fixtures, and demo capabilities for testing
//! [Everruns](https://everruns.com) agents.
//!
//! `everruns-test-support` is the testing and demonstration companion to the
//! production-safe [`everruns-llmsim`](https://docs.rs/everruns-llmsim) crate.
//! It keeps reusable helpers out of production compositions:
//!
//! - [`in_memory_loop`] — a full in-memory agentic loop with no database or
//!   network (`sim` + `host` features)
//! - [`in_memory`] — deterministic message and event fixtures for isolated
//!   tests; hosted loops use canonical event history instead
//! - [`doubles`] — mock/echo/failing tool executors and a mock chat driver
//! - [`capabilities`] — fake AWS/CRM/financial/warehouse demo capabilities
//!   and the test math/weather, sample-data, and noop fixtures
//!   (`fixtures` feature)
//! - a source-compatible 0.18 migration bridge for the simulator types and
//!   host extension previously published here, including registration-only
//!   `.llm_sim(...)` and explicit `.llm_sim_as_default(...)` (`sim` / `host`
//!   features)
//!
//! Production code should use `everruns::Model::simulated` or depend on
//! `everruns-llmsim` directly. Do not add a production dependency on this
//! crate or register its fixture capabilities.
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

/// Compatibility module preserving the 0.17 simulator path during the 0.18
/// migration.
///
/// New production code should import these items from `everruns_llmsim`.
#[cfg(feature = "sim")]
pub mod llmsim_driver {
    pub use everruns_llmsim::*;
}

// In-memory agentic loop built on the llmsim driver.
#[cfg(all(feature = "sim", feature = "host"))]
pub mod in_memory_loop;

pub mod in_memory;

// Test doubles for the core execution traits.
pub mod doubles;

// Demo/test fixture capabilities (behind the `fixtures` feature).
#[cfg(feature = "fixtures")]
pub mod capabilities;

#[cfg(feature = "host")]
mod store_reason_atom;

pub use doubles::{
    EchoToolExecutor, FailingToolExecutor, MockLlmResponse, MockProvider, MockToolExecutor,
};
#[cfg(feature = "sim")]
pub use everruns_llmsim::{
    LlmSimConfig, LlmSimDriver, OnExhausted, ResponseConfig, SimError, SimToolCall, SimTurn,
    ToolCallConfig, ToolCallPattern,
};
pub use everruns_builtins::InMemoryMessageRetriever;
pub use in_memory::InMemoryEventEmitter;
#[cfg(all(feature = "sim", feature = "host"))]
pub use in_memory_loop::{InMemoryAgenticLoop, InMemoryAgenticLoopBuilder};

#[cfg(feature = "fixtures")]
pub use capabilities::{
    FAKE_AWS_CAPABILITY_ID, FAKE_CRM_CAPABILITY_ID, FAKE_FINANCIAL_CAPABILITY_ID,
    FAKE_WAREHOUSE_CAPABILITY_ID, FakeAwsCapability, FakeCrmCapability, FakeFinancialCapability,
    FakeWarehouseCapability, NOOP_CAPABILITY_ID, NoopCapability, SAMPLE_DATA_CAPABILITY_ID,
    SampleDataCapability, TEST_MATH_CAPABILITY_ID, TEST_WEATHER_CAPABILITY_ID, TestMathCapability,
    TestWeatherCapability,
};

#[cfg(feature = "host")]
pub use everruns_llmsim::{LLMSIM_MODEL_ID, LLMSIM_PROVIDER, LlmSimRuntimeExt, llm_sim_provider};

#[cfg(feature = "host")]
pub use store_reason_atom::reason_atom_with_stores;
