// Vendor-neutral error reporting hook for embedding.
//
// Decision: Stay vendor-neutral — no Sentry, Datadog, or Rollbar types leak
//   into OSS. Wrappers map `ErrorReport` onto whatever backend they use.
// Decision: Separate the "report this error" contract (ErrorReporter) from
//   the "where did it happen" context (ErrorScope). The scope is structured
//   so wrappers can attach tags/breadcrumbs without string-parsing.
// Decision: Async trait so reporters can perform IO (HTTP, channel send) if
//   they want to. Implementations should be best-effort and swallow errors —
//   a failing reporter must never propagate into the request/task path.

use async_trait::async_trait;
use std::collections::BTreeMap;
use std::sync::Arc;

/// Severity of the reported error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ErrorSeverity {
    /// Unexpected but non-fatal condition.
    Warning,
    /// Handled error that did not crash the process.
    Error,
    /// Unhandled panic or fatal condition.
    Fatal,
}

/// Structured request/task scope surrounding the error.
///
/// Every field is optional because the surrounding context differs between
/// HTTP requests, worker tasks, and background jobs. Wrappers map these
/// onto vendor-specific tag/context APIs (e.g. Sentry scope tags).
#[derive(Debug, Clone, Default)]
pub struct ErrorScope {
    pub user_id: Option<String>,
    pub org_id: Option<String>,
    pub session_id: Option<String>,
    pub request_id: Option<String>,
    pub route: Option<String>,
    pub component: Option<String>,
    pub task_id: Option<String>,
    pub workflow_id: Option<String>,
    /// Extra key/value metadata (provider id, feature flag, etc.).
    pub extra: BTreeMap<String, String>,
}

impl ErrorScope {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_user(mut self, user_id: impl Into<String>) -> Self {
        self.user_id = Some(user_id.into());
        self
    }

    pub fn with_org(mut self, org_id: impl Into<String>) -> Self {
        self.org_id = Some(org_id.into());
        self
    }

    pub fn with_session(mut self, session_id: impl Into<String>) -> Self {
        self.session_id = Some(session_id.into());
        self
    }

    pub fn with_request(mut self, request_id: impl Into<String>) -> Self {
        self.request_id = Some(request_id.into());
        self
    }

    pub fn with_route(mut self, route: impl Into<String>) -> Self {
        self.route = Some(route.into());
        self
    }

    pub fn with_component(mut self, component: impl Into<String>) -> Self {
        self.component = Some(component.into());
        self
    }

    pub fn with_task(mut self, task_id: impl Into<String>) -> Self {
        self.task_id = Some(task_id.into());
        self
    }

    pub fn with_workflow(mut self, workflow_id: impl Into<String>) -> Self {
        self.workflow_id = Some(workflow_id.into());
        self
    }

    pub fn with_extra(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.extra.insert(key.into(), value.into());
        self
    }
}

/// A single error report passed to the embedder-provided reporter.
#[derive(Debug, Clone)]
pub struct ErrorReport {
    pub severity: ErrorSeverity,
    /// Short machine-stable identifier (e.g. `"worker.task.failed"`,
    /// `"server.request.panic"`). Vendor-neutral.
    pub kind: String,
    /// Human-readable error message.
    pub message: String,
    pub scope: ErrorScope,
}

impl ErrorReport {
    pub fn new(
        severity: ErrorSeverity,
        kind: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            severity,
            kind: kind.into(),
            message: message.into(),
            scope: ErrorScope::default(),
        }
    }

    pub fn error(kind: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(ErrorSeverity::Error, kind, message)
    }

    pub fn warning(kind: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(ErrorSeverity::Warning, kind, message)
    }

    pub fn fatal(kind: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(ErrorSeverity::Fatal, kind, message)
    }

    pub fn with_scope(mut self, scope: ErrorScope) -> Self {
        self.scope = scope;
        self
    }
}

/// Vendor-neutral error reporting hook.
///
/// Implementations are supplied by embedders (SaaS wrappers) via
/// `ServerAppBuilder::error_reporter` / `WorkerAppBuilder::error_reporter`.
/// OSS never imports vendor-specific SDKs (Sentry, Datadog, etc.).
///
/// Implementations must be best-effort: a slow or failing reporter must
/// never propagate into the request or task execution path. Spawn background
/// work for heavy reporting if needed.
#[async_trait]
pub trait ErrorReporter: Send + Sync {
    /// Deliver a report to the embedder-owned backend.
    async fn report(&self, report: ErrorReport);

    /// Human-readable name for logging and diagnostics.
    fn name(&self) -> &'static str {
        "ErrorReporter"
    }
}

/// Convenience type for handing a reporter to consumers.
pub type SharedErrorReporter = Arc<dyn ErrorReporter>;

/// No-op reporter. Default when no embedder reporter is installed.
#[derive(Debug, Clone, Default)]
pub struct NoopErrorReporter;

#[async_trait]
impl ErrorReporter for NoopErrorReporter {
    async fn report(&self, _report: ErrorReport) {}

    fn name(&self) -> &'static str {
        "NoopErrorReporter"
    }
}
