//! Platform REST migration dates shared by the control plane and edge.

/// Legacy management URLs become deprecated on December 22, 2026 (Chicago).
/// This announces deprecation, not a scheduled removal or automatic shutdown.
pub const LEGACY_DEPRECATION: &str = "@1797919200";
pub const LEGACY_DEPRECATION_DATE: &str = "2026-12-22";
pub const MIGRATION_GUIDE: &str =
    "https://everruns.com/docs/changelog/2026-11-22-platform-api-namespace/";
