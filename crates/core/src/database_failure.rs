// Shared classification for storage failures that strike many subsystems at
// once.
//
// Decision: classify from the *rendered* error text, not from a driver error
//   type. By the time a background poller logs one of these, the original
//   error has usually been flattened into a string-carrying variant of the
//   subsystem's own error enum, so a type-based classifier would only reach a
//   fraction of the call sites that need it. Text is the one representation
//   every site still has — and a driver type would drag a database dependency
//   into the kernel, which EVE-903 forbids.
// Decision: stay vendor-neutral, like `error_reporter`. This module names no
//   Sentry concept; it produces a stable kind and a canonical message, and the
//   embedder's reporter decides what a "fingerprint" means.
//
// EVE-1071: a connection-pool exhaustion in prod surfaced as four separate
// Sentry issues, one per background poller, because each poller rendered the
// same underlying failure into its own message. Grouping is driven by the
// message, so "one incident, one issue" requires the subsystems to agree on
// the wording and carry their identity in a field instead.

/// A storage failure, classified into the shape that decides how it is
/// reported.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DatabaseFailureKind {
    /// No connection became available within the pool's acquire timeout.
    ///
    /// Almost never a defect in the subsystem that reports it: the reporter is
    /// starved by whatever else is holding connections.
    PoolExhausted,
    /// The connection went away mid-flight — reset, closed, or the server
    /// terminated it.
    ConnectionLost,
    /// Anything else, including ordinary query and constraint errors.
    Other,
}

impl DatabaseFailureKind {
    /// Classify a failure from its rendered text.
    ///
    /// Matching is substring-based and case-insensitive because the text
    /// arrives wrapped in however many layers of context each call site added.
    pub fn classify(rendered: &str) -> Self {
        let text = rendered.to_ascii_lowercase();
        // The driver renders an acquire timeout as exactly this sentence; the
        // shorter "pool timed out" guards against a wrapper that truncated it.
        if text.contains("pool timed out while waiting for an open connection")
            || text.contains("pool timed out")
        {
            return Self::PoolExhausted;
        }
        if text.contains("connection reset by peer")
            || text.contains("connection closed")
            || text.contains("error communicating with database")
            || text.contains("server closed the connection unexpectedly")
        {
            return Self::ConnectionLost;
        }
        Self::Other
    }

    /// Stable, machine-readable identifier for this kind.
    ///
    /// Suitable as an `ErrorReport::kind` and as a log field value.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::PoolExhausted => "database.pool_exhausted",
            Self::ConnectionLost => "database.connection_lost",
            Self::Other => "database.other",
        }
    }

    /// Whether this failure is one shared incident rather than a fault in the
    /// subsystem that noticed it.
    ///
    /// These report under [`Self::shared_message`] so that every starved
    /// subsystem lands in one group, with its own identity in the `subsystem`
    /// field. Everything else keeps the call site's own message, which is what
    /// makes an ordinary query bug findable.
    pub fn is_shared_incident(&self) -> bool {
        matches!(self, Self::PoolExhausted | Self::ConnectionLost)
    }

    /// The canonical message every subsystem uses for a shared incident.
    ///
    /// `None` for [`Self::Other`], which has no shared wording to agree on.
    pub fn shared_message(&self) -> Option<&'static str> {
        match self {
            Self::PoolExhausted => Some("database connection pool exhausted"),
            Self::ConnectionLost => Some("database connection lost"),
            Self::Other => None,
        }
    }
}

/// Log a storage failure so that one infrastructure incident reads as one
/// incident, and report how it was classified.
///
/// `subsystem` names who hit the failure and `fallback` is the message for an
/// ordinary failure, where the call site is the interesting thing. A shared
/// incident overrides `fallback` with [`DatabaseFailureKind::shared_message`]
/// so every starved subsystem groups together.
///
/// Both message arguments render to a constant, which is what a log pipeline
/// groups on; the varying part stays in the `error` field.
pub fn log_database_failure(
    subsystem: &'static str,
    fallback: &'static str,
    rendered: &str,
) -> DatabaseFailureKind {
    let kind = DatabaseFailureKind::classify(rendered);
    match kind.shared_message() {
        Some(shared) => tracing::error!(
            subsystem,
            db_failure = kind.as_str(),
            error = %rendered,
            "{shared}"
        ),
        None => tracing::error!(
            subsystem,
            db_failure = kind.as_str(),
            error = %rendered,
            "{fallback}"
        ),
    }
    kind
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verbatim from the Sentry events in EVE-1071, so the classifier is
    /// pinned against what production actually produced rather than against
    /// what the driver's source suggests it produces.
    const PROD_POOL_TIMEOUT: &str =
        "Failed to claim due schedules: pool timed out while waiting for an open connection";
    const PROD_CONNECTION_RESET: &str = "Failed to claim due schedules: error communicating with database: Connection reset by peer (os error 104)";
    const PROD_CLAIM_TASKS_RESET: &str = "Failed to claim tasks: database error: error communicating with database: Connection reset by peer (os error 104)";

    #[test]
    fn the_prod_pool_timeout_classifies_as_exhaustion() {
        assert_eq!(
            DatabaseFailureKind::classify(PROD_POOL_TIMEOUT),
            DatabaseFailureKind::PoolExhausted
        );
    }

    #[test]
    fn the_prod_connection_resets_classify_as_a_lost_connection() {
        for rendered in [PROD_CONNECTION_RESET, PROD_CLAIM_TASKS_RESET] {
            assert_eq!(
                DatabaseFailureKind::classify(rendered),
                DatabaseFailureKind::ConnectionLost,
                "{rendered}"
            );
        }
    }

    #[test]
    fn every_starved_subsystem_agrees_on_one_message() {
        // The point of the whole module: four pollers, four wordings, one
        // incident. Whatever context each added, they must land on the same
        // shared message. (That the *loggers* honour it is covered in
        // `everruns-durable`, which can hold a subscriber; EVE-876 keeps one
        // out of this crate.)
        let renderings = [
            PROD_POOL_TIMEOUT,
            "Failed to claim tasks: pool timed out while waiting for an open connection",
            "failed to process due schedules: pool timed out while waiting for an open connection",
            "Observer scoring batch failed: pool timed out while waiting for an open connection",
        ];
        let messages: std::collections::BTreeSet<_> = renderings
            .iter()
            .map(|rendered| DatabaseFailureKind::classify(rendered).shared_message())
            .collect();
        assert_eq!(
            messages,
            std::collections::BTreeSet::from([Some("database connection pool exhausted")])
        );
    }

    #[test]
    fn classification_ignores_case_and_surrounding_context() {
        assert_eq!(
            DatabaseFailureKind::classify("Store(Database(\"POOL TIMED OUT\"))"),
            DatabaseFailureKind::PoolExhausted
        );
    }

    #[test]
    fn an_ordinary_query_error_keeps_the_call_sites_own_message() {
        let kind = DatabaseFailureKind::classify(
            "duplicate key value violates unique constraint \"sessions_pkey\"",
        );
        assert_eq!(kind, DatabaseFailureKind::Other);
        assert!(!kind.is_shared_incident());
        assert_eq!(kind.shared_message(), None);
    }

    #[test]
    fn shared_incidents_are_the_ones_with_shared_wording() {
        for kind in [
            DatabaseFailureKind::PoolExhausted,
            DatabaseFailureKind::ConnectionLost,
            DatabaseFailureKind::Other,
        ] {
            assert_eq!(
                kind.is_shared_incident(),
                kind.shared_message().is_some(),
                "{kind:?}"
            );
        }
    }

    #[test]
    fn kinds_have_distinct_stable_identifiers() {
        let ids: std::collections::BTreeSet<_> = [
            DatabaseFailureKind::PoolExhausted,
            DatabaseFailureKind::ConnectionLost,
            DatabaseFailureKind::Other,
        ]
        .iter()
        .map(|kind| kind.as_str())
        .collect();
        assert_eq!(ids.len(), 3);
        assert_eq!(
            DatabaseFailureKind::PoolExhausted.as_str(),
            "database.pool_exhausted"
        );
    }

    #[test]
    fn logging_reports_the_classification_it_used() {
        assert_eq!(
            log_database_failure(
                "durable.tasks.claim",
                "Failed to claim tasks",
                PROD_POOL_TIMEOUT
            ),
            DatabaseFailureKind::PoolExhausted
        );
        assert_eq!(
            log_database_failure(
                "durable.tasks.claim",
                "Failed to claim tasks",
                "no such row"
            ),
            DatabaseFailureKind::Other
        );
    }
}
