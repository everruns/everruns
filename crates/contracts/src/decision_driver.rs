//! Decision drivers: the vendor seam behind [`DecisionsService`].
//!
//! A decision driver is to the decisions service what a chat driver is to a
//! model provider: one vendor's transport, answering the provider-neutral
//! [`DecisionRequest`]. Callers keep talking to a [`DecisionsService`]; the
//! host puts a router in front of a registry of drivers (see
//! `crate::runtime_provider::ProviderRegistry`), so a deployment switches vendors
//! without touching a call site.
//!
//! Decisions recorded here:
//!
//! - A driver answers all three primitives. One whose vendor lacks a primitive
//!   translates it (a `Noul` as a two-option choice, a `Score` as a choice over
//!   the levels) and says so through [`DecisionDriverCapabilities::native`].
//!   Callers never see a primitive rejected for being foreign to a vendor.
//! - A driver whose vendor returns a label, not a distribution, reports
//!   `DecisionOutcome::calibrated = false` and encodes the label one-hot with
//!   the `DecisionAnswer::*_label` constructors. It never invents the numbers
//!   in between.
//! - Limits are declared, not discovered: the router checks a request against
//!   them before any round trip, so an oversized request is a clear local
//!   error rather than a vendor 400.

use async_trait::async_trait;

use crate::decisions::{DecisionOutcome, DecisionQuestion, DecisionRequest, DecisionsService};
use crate::error::{AgentLoopError, Result};

/// Which primitives a vendor answers without translation.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NativePrimitives {
    /// Yes/no as a probability.
    pub noul: bool,
    /// One option from a set.
    pub choice: bool,
    /// A position along ordered levels.
    pub score: bool,
}

impl NativePrimitives {
    /// All three primitives are native.
    pub const ALL: Self = Self {
        noul: true,
        choice: true,
        score: true,
    };

    /// Only `choice` is native; the driver translates the other two.
    pub const CHOICE_ONLY: Self = Self {
        noul: false,
        choice: true,
        score: false,
    };

    /// Whether `question` is answered without translation.
    pub fn covers(&self, question: &DecisionQuestion) -> bool {
        match question {
            DecisionQuestion::Noul { .. } => self.noul,
            DecisionQuestion::Choice { .. } => self.choice,
            DecisionQuestion::Score { .. } => self.score,
        }
    }
}

/// What a driver can do, declared up front.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct DecisionDriverCapabilities {
    /// Primitives the vendor answers natively. The rest are translated.
    pub native: NativePrimitives,
    /// Whether answers are measured distributions rather than labels.
    pub calibrated: bool,
    /// Whether the vendor accepts image content in the state.
    pub image_state: bool,
    /// Most questions one request may carry, when the vendor caps it.
    pub max_questions: Option<usize>,
    /// Most options (or levels) one question may carry.
    pub max_options: Option<usize>,
    /// Primitive-specific serving limits.
    pub max_choice_options: Option<usize>,
    pub max_score_levels: Option<usize>,
    /// Most bytes of serialized state one request may carry.
    pub max_state_bytes: Option<usize>,
}

impl DecisionDriverCapabilities {
    /// Capabilities with the given native primitives and calibration, no
    /// image input, and no declared limits.
    pub fn new(native: NativePrimitives, calibrated: bool) -> Self {
        Self {
            native,
            calibrated,
            ..Self::default()
        }
    }

    /// Declare image support.
    pub fn with_image_state(mut self, image_state: bool) -> Self {
        self.image_state = image_state;
        self
    }

    /// Declare the per-request question cap.
    pub fn with_max_questions(mut self, max: usize) -> Self {
        self.max_questions = Some(max);
        self
    }

    /// Declare the per-question option (or level) cap.
    pub fn with_max_options(mut self, max: usize) -> Self {
        self.max_options = Some(max);
        self
    }

    pub fn with_max_choice_options(mut self, max: usize) -> Self {
        self.max_choice_options = Some(max);
        self
    }
    pub fn with_max_score_levels(mut self, max: usize) -> Self {
        self.max_score_levels = Some(max);
        self
    }

    /// Declare the per-request state size cap.
    pub fn with_max_state_bytes(mut self, max: usize) -> Self {
        self.max_state_bytes = Some(max);
        self
    }

    /// Reject `request` locally when it exceeds a declared limit.
    pub fn check(&self, driver: &str, request: &DecisionRequest) -> Result<()> {
        if request.is_empty() {
            return Err(AgentLoopError::llm(
                "decision request must carry at least one question",
            ));
        }
        if let Some(max) = self.max_questions
            && request.len() > max
        {
            return Err(AgentLoopError::llm(format!(
                "decision driver '{driver}' accepts at most {max} questions per request, got {}",
                request.len()
            )));
        }
        if let Some(max) = self.max_options {
            for (id, question) in &request.questions {
                let count = match question {
                    DecisionQuestion::Noul { .. } => 2,
                    DecisionQuestion::Choice { options, .. } => options.len(),
                    DecisionQuestion::Score { levels, .. } => levels.len(),
                };
                if count > max {
                    return Err(AgentLoopError::llm(format!(
                        "decision driver '{driver}' accepts at most {max} options per question; \
                         '{id}' has {count}"
                    )));
                }
            }
        }
        for (_, question) in &request.questions {
            let (count, max) = match question {
                DecisionQuestion::Choice { options, .. } => {
                    (options.len(), self.max_choice_options)
                }
                DecisionQuestion::Score { levels, .. } => (levels.len(), self.max_score_levels),
                _ => continue,
            };
            if max.is_some_and(|max| count > max) {
                return Err(AgentLoopError::llm(
                    "Decision question exceeds primitive limit",
                ));
            }
        }
        if let Some(max) = self.max_state_bytes {
            let bytes = match &request.state {
                serde_json::Value::String(text) => text.len(),
                other => other.to_string().len(),
            };
            if bytes > max {
                return Err(AgentLoopError::llm(format!(
                    "decision driver '{driver}' accepts at most {max} bytes of state, got {bytes}"
                )));
            }
        }
        Ok(())
    }
}

/// One vendor's transport for typed decisions.
///
/// Credentials belong to the driver and never appear in a request: a driver is
/// composed by the deployment or the embedding application
/// (THREAT[TM-LLM-037]).
#[async_trait]
pub trait DecisionDriver: Send + Sync {
    /// Stable driver id, as used in `driver/model` routing and in
    /// `UTILITY_DECISION_DRIVER`: `typesafe`, `openai`, `llm`.
    fn id(&self) -> &str;

    /// What this driver answers and within which limits.
    fn capabilities(&self) -> DecisionDriverCapabilities;

    /// Answer every question in `request`.
    ///
    /// `request.model` is opaque. The selected provider owns explicit aliases;
    /// the router never interprets model namespaces as provider keys.
    async fn evaluate(
        &self,
        endpoint: &crate::runtime_provider::ProviderEndpoint,
        request: DecisionRequest,
    ) -> Result<DecisionOutcome>;
}

/// Serve a single driver as a [`DecisionsService`].
///
/// For embedders that hold one vendor and want no registry: the Framework's
/// `Decisions::new(model, driver)` takes any service, and this makes a driver
/// one.
pub struct SingleDriverService<D>(pub D);

#[async_trait]
impl<D: DecisionDriver> DecisionsService for SingleDriverService<D> {
    fn is_configured(&self) -> bool {
        true
    }

    async fn evaluate(&self, request: DecisionRequest) -> Result<DecisionOutcome> {
        if request
            .provider
            .as_ref()
            .is_some_and(|provider| provider.as_str() != self.0.id())
        {
            return Err(AgentLoopError::Configuration(
                "Selected provider is unavailable in this service".into(),
            ));
        }
        self.0.capabilities().check(self.0.id(), &request)?;
        self.0
            .evaluate(
                &crate::runtime_provider::ProviderEndpoint::default(),
                request,
            )
            .await
    }

    fn name(&self) -> &'static str {
        "SingleDriverService"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request_with(questions: usize) -> DecisionRequest {
        (0..questions).fold(DecisionRequest::new("state"), |request, index| {
            request.ask(format!("q{index}"), DecisionQuestion::noul("Yes?"))
        })
    }

    #[test]
    fn limits_reject_oversized_requests_before_any_round_trip() {
        let caps = DecisionDriverCapabilities::new(NativePrimitives::ALL, true)
            .with_max_questions(2)
            .with_max_options(3)
            .with_max_state_bytes(10);
        assert!(caps.check("x", &request_with(2)).is_ok());
        let error = caps.check("x", &request_with(3)).unwrap_err();
        assert!(error.to_string().contains("at most 2 questions"), "{error}");

        let wide = DecisionRequest::new("s")
            .ask("q", DecisionQuestion::score("How?", ["a", "b", "c", "d"]));
        let error = caps.check("x", &wide).unwrap_err();
        assert!(error.to_string().contains("'q' has 4"), "{error}");

        let long = DecisionRequest::new("far more than ten bytes")
            .ask("q", DecisionQuestion::noul("Yes?"));
        let error = caps.check("x", &long).unwrap_err();
        assert!(error.to_string().contains("bytes of state"), "{error}");

        let error = caps.check("x", &DecisionRequest::new("s")).unwrap_err();
        assert!(error.to_string().contains("at least one question"));
    }

    #[test]
    fn native_primitives_cover_by_kind() {
        let noul = DecisionQuestion::noul("Yes?");
        let score = DecisionQuestion::score("How?", ["a", "b"]);
        assert!(NativePrimitives::ALL.covers(&noul));
        assert!(!NativePrimitives::CHOICE_ONLY.covers(&noul));
        assert!(!NativePrimitives::CHOICE_ONLY.covers(&score));
    }
}
