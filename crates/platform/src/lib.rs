//! Backend platform identity entities for Everruns.
//!
//! `everruns-platform` owns the durable backend identity aggregates that were
//! historically defined in [`everruns-core`](https://docs.rs/everruns-core):
//! [`Organization`] and [`Principal`]. It was carved out in EVE-837 to give the
//! backend identity family its own crate without disturbing the runtime-facing
//! contract in `everruns-core`.
//!
//! # Dependency direction
//!
//! The dependency edge is strictly `platform -> core`. `everruns-core` never
//! depends on `everruns-platform`; that invariant is enforced by
//! `crates/core/tests/no_platform_dependency.rs`.
//!
//! # Types that stay in core
//!
//! Cross-cutting value types that core's runtime, permissions layer, and domain
//! models embed remain in `everruns-core` and are re-exported here so consumers
//! can reach the whole identity surface through `everruns_platform`:
//!
//! - [`OrgRole`], [`OrgMembership`] and the multitenancy constants — used by
//!   core's permissions/auth layer.
//! - [`PrincipalKind`], [`PrincipalStatus`], [`PrincipalSummary`] — embedded in
//!   `Session`/`App`/`SessionSchedule` and the agent-identity lifecycle.

pub mod organization;
pub mod principal;

pub use organization::Organization;
pub use principal::Principal;

// Re-export the identity value types and multitenancy constants that remain in
// `everruns-core`, so `everruns_platform` exposes the complete identity surface.
pub use everruns_core::{
    ANONYMOUS_USER_EMAIL, ANONYMOUS_USER_ID, ANONYMOUS_USER_NAME, DEFAULT_ORG_ID,
    DEFAULT_ORG_PUBLIC_ID, OrgMembership, OrgRole, PrincipalKind, PrincipalStatus,
    PrincipalSummary, generate_org_public_id, org_public_id_from_internal, validate_org_public_id,
};
