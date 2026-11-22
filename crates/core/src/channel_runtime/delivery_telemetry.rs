//! The transport-neutral delivery observation port.

use std::sync::Arc;
use std::time::SystemTime;

use super::DeliveryResult;

/// Whether the platform accepted the delivery, or how it failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeliveryOutcome {
    Succeeded,
    Transient,
    Permanent,
}

impl DeliveryOutcome {
    /// Fixed error category; never a transport error message.
    pub fn error_type(self) -> Option<&'static str> {
        match self {
            Self::Succeeded => None,
            Self::Transient => Some("transient_delivery"),
            Self::Permanent => Some("permanent_delivery"),
        }
    }

    pub(super) fn or(self, next: Self) -> Self {
        if self.error_type().is_some() {
            self
        } else {
            next
        }
    }
}

impl From<&DeliveryResult> for DeliveryOutcome {
    fn from(result: &DeliveryResult) -> Self {
        match result {
            DeliveryResult::Ok => Self::Succeeded,
            DeliveryResult::TransientError(_) => Self::Transient,
            DeliveryResult::PermanentError(_) => Self::Permanent,
        }
    }
}

/// The kind of generated output sent to the platform.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeliveryKind {
    Approval,
    Reply,
}

/// Receives content-free results for one turn's outbound delivery attempts.
pub trait DeliveryObserver: Send + Sync {
    /// Observe an attempt under the committed host-owned turn trace carrier.
    fn observe(
        &self,
        platform: &str,
        traceparent: &str,
        kind: DeliveryKind,
        outcome: DeliveryOutcome,
        start: SystemTime,
    );
}

#[derive(Default)]
pub(super) struct DeliveryTelemetry {
    pub(super) observer: Option<Arc<dyn DeliveryObserver>>,
    anchor: Option<String>,
}

impl DeliveryTelemetry {
    pub(super) fn anchor(&mut self, carrier: &str) {
        if self.observer.is_some() && self.anchor.is_none() {
            self.anchor = Some(carrier.to_owned());
        }
    }

    pub(super) fn start(&self) -> Option<SystemTime> {
        self.anchor.as_ref().map(|_| SystemTime::now())
    }

    pub(super) fn finish(
        &self,
        platform: &str,
        start: Option<SystemTime>,
        kind: DeliveryKind,
        outcome: DeliveryOutcome,
    ) {
        let Some(observer) = &self.observer else {
            return;
        };
        let Some(anchor) = &self.anchor else { return };
        if let Some(start) = start {
            observer.observe(platform, anchor, kind, outcome, start);
        }
    }
}
