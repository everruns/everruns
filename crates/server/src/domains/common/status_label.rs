// Metric status labels for command outcomes, split out of `common.rs`.

use super::{CommandError, CommandErrorKind};

/// Stable, low-cardinality status label for the `everruns_commands_total`
/// counter and `everruns_command_duration_seconds` histogram.
pub(super) fn command_error_status_label(err: &CommandError) -> &'static str {
    match err {
        CommandError {
            kind: CommandErrorKind::BadRequest(_),
            ..
        } => "bad_request",
        CommandError {
            kind: CommandErrorKind::Unprocessable(_),
            ..
        } => "unprocessable",
        CommandError {
            kind: CommandErrorKind::Forbidden(_),
            ..
        } => "forbidden",
        CommandError {
            kind: CommandErrorKind::NotFound(_),
            ..
        } => "not_found",
        CommandError {
            kind: CommandErrorKind::Conflict(_),
            ..
        } => "conflict",
        CommandError {
            kind: CommandErrorKind::RateLimited(_),
            ..
        } => "rate_limited",
        CommandError {
            kind: CommandErrorKind::Unavailable(_),
            ..
        } => "unavailable",
        CommandError {
            kind: CommandErrorKind::Internal(_),
            ..
        } => "internal",
    }
}
