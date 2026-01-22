//! Context Strategy Capabilities for Evaluation
//!
//! These capabilities implement context management strategies using the
//! core infrastructure (Capability trait, MessageFilterProvider, etc.)
//! but live in the evals crate for research purposes.

pub mod naive_trim;
mod infinity_context;

pub use naive_trim::NaiveTrimCapability;
pub use infinity_context::{InfinityContextCapability, QueryHistoryTool};

// Re-export the config from types
pub use crate::types::ContextStrategyConfig;

// Re-export useful traits from core
pub use everruns_core::capabilities::{Capability, CapabilityStatus};
pub use everruns_core::message_filter::{
    BatchTransformResult, InjectedMessage, InjectionPosition, MessageFilter, MessageFilterProvider,
    MessageQuery,
};
