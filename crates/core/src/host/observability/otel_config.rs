//! OpenTelemetry listener configuration.

/// Which attribute vocabularies the listener writes on its spans.
///
/// Both are on by default so one OTLP stream renders in Gen-AI-aware
/// backends and in Phoenix. Narrow with `EVERRUNS_TRACE_CONVENTIONS`
/// (`gen_ai`, `openinference`, or both, comma-separated).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TraceConventions {
    /// OpenTelemetry Gen-AI semantic conventions (`gen_ai.*`).
    pub gen_ai: bool,
    /// OpenInference conventions (`openinference.span.kind`, `llm.*`, ...).
    pub openinference: bool,
}

impl TraceConventions {
    pub const ALL: Self = Self {
        gen_ai: true,
        openinference: true,
    };
    pub const GEN_AI: Self = Self {
        gen_ai: true,
        openinference: false,
    };
    pub const OPENINFERENCE: Self = Self {
        gen_ai: false,
        openinference: true,
    };

    /// Environment variable selecting the conventions.
    pub const ENV: &'static str = "EVERRUNS_TRACE_CONVENTIONS";

    pub fn from_env() -> Self {
        std::env::var(Self::ENV)
            .ok()
            .map(|value| Self::parse(&value))
            .unwrap_or(Self::ALL)
    }

    /// Parse a comma-separated list. Unknown or empty selections fall back
    /// to every convention, so a typo widens rather than silences telemetry.
    pub fn parse(value: &str) -> Self {
        let mut selected = Self {
            gen_ai: false,
            openinference: false,
        };
        for token in value.split(',').map(|t| t.trim().to_ascii_lowercase()) {
            match token.as_str() {
                "gen_ai" | "genai" | "otel" | "opentelemetry" => selected.gen_ai = true,
                "openinference" | "oi" | "phoenix" => selected.openinference = true,
                "all" | "both" => return Self::ALL,
                "" => {}
                other => tracing::warn!(value = other, "Unknown {} entry, ignoring", Self::ENV),
            }
        }
        if selected
            == (Self {
                gen_ai: false,
                openinference: false,
            })
        {
            Self::ALL
        } else {
            selected
        }
    }
}

impl Default for TraceConventions {
    fn default() -> Self {
        Self::ALL
    }
}

/// Read the content-capture opt-in. Accepts the boolean spelling and the
/// mode names used by the reference Python instrumentation.
///
/// THREAT[TM-OBS-010]: prompts, completions, reasoning, and tool payloads only
/// reach the OTLP endpoint when the operator turns this on; every content
/// attribute below is gated on the resulting `record_content` flag.
pub(super) fn record_content_from_env() -> bool {
    let value = std::env::var("OTEL_INSTRUMENTATION_GENAI_CAPTURE_MESSAGE_CONTENT")
        .or_else(|_| std::env::var("OTEL_RECORD_CONTENT"))
        .unwrap_or_default();
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "true" | "1" | "yes" | "on" | "span_only" | "span_and_event" | "event_only"
    )
}
