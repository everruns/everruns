//! OpenAI Agents API runtime selection (EVE-1123).
//!
//! A marker capability: it adds no tools or prompt. Its presence in a turn's
//! resolved capabilities makes the host run the turn's loop through OpenAI's
//! Agents API (`everruns_core::host::openai_agents_api`) instead of the native
//! Reason loop. The platform strips it from the worker snapshot unless the org
//! has the platform-managed `openai_agents_api` flag, so the default path is
//! unchanged.

use super::{Capability, CapabilityStatus, RiskLevel};
use everruns_core::capabilities::OPENAI_AGENTS_API_RUNTIME_ID;

pub struct OpenAiAgentsApiRuntimeCapability;

impl Capability for OpenAiAgentsApiRuntimeCapability {
    fn localizations(&self) -> Vec<everruns_core::capabilities::CapabilityLocalization> {
        vec![everruns_core::capabilities::CapabilityLocalization::text(
            "uk",
            "Виконання через OpenAI Agents API",
            "Виконує цикл агента через OpenAI Agents API. Інструменти Everruns працюють через звичайний конвеєр. Дані сесії обробляє OpenAI; ці сесії не підтримують нульове зберігання даних.",
        )]
    }

    fn id(&self) -> &str {
        OPENAI_AGENTS_API_RUNTIME_ID
    }

    fn name(&self) -> &str {
        "OpenAI Agents API Runtime"
    }

    fn description(&self) -> &str {
        "Run this agent's loop on OpenAI's Agents API. Everruns tools run as client \
         functions through the normal tool pipeline; OpenAI built-ins and direct MCP \
         are not enabled. Requires a model on the official OpenAI API; other models \
         keep the native runtime. Session data is processed by OpenAI, and Agents API \
         sessions do not qualify for Zero Data Retention."
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn icon(&self) -> Option<&str> {
        Some("cloud")
    }

    fn category(&self) -> Option<&str> {
        Some("Runtime")
    }

    /// Conversation content and tool results leave the deployment for a
    /// third-party managed loop without Zero Data Retention.
    fn risk_level(&self) -> RiskLevel {
        RiskLevel::High
    }
}
