#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//! Everruns model drivers for vendors that need no bespoke wire protocol.
//!
//! `everruns-drivers` is part of the [Everruns](https://everruns.com)
//! ecosystem. Each module here implements the [`ChatDriver`] contract from
//! `everruns-provider` by wrapping one of that crate's shared protocol drivers
//! — `OpenAIProtocolChatDriver` (Chat Completions) or
//! `OpenResponsesProtocolChatDriver` (Open Responses) — and adding only what is
//! vendor-specific: identity, credential schema, authentication, base URL, and
//! model discovery.
//!
//! # Membership
//!
//! A vendor belongs here when a shared protocol driver already speaks its wire.
//! It graduates to its own crate when it needs its own request and response
//! types, or dependencies heavy enough that consumers of the other vendors
//! should not pay for them.
//!
//! # Features
//!
//! Every vendor is behind a feature and none is on by default, so a consumer
//! compiles and ships only the vendors it serves:
//!
//! ```toml
//! everruns-drivers = { version = "0.29.0", features = ["cloudflare", "vercel"] }
//! ```
//!
//! | Feature | Driver | Wire protocol |
//! | --- | --- | --- |
//! | `cloudflare` | Cloudflare AI Gateway | OpenAI Chat Completions (`/compat`) |
//! | `vercel` | Vercel AI Gateway | Open Responses |

#[cfg(feature = "cloudflare")]
pub mod cloudflare;
#[cfg(feature = "vercel")]
pub mod vercel;

// Re-export core types for convenience.
pub use everruns_provider::driver_registry::{ChatDriver, DriverRegistry};

/// Register every driver this crate's enabled features provide.
///
/// A host that enables a feature gets that vendor without editing its
/// registration code, which is the point of keeping them in one crate.
///
/// # Example
///
/// ```
/// use everruns_drivers::{register_drivers, DriverRegistry};
///
/// let mut registry = DriverRegistry::new();
/// register_drivers(&mut registry);
/// ```
#[cfg_attr(
    not(any(feature = "cloudflare", feature = "vercel")),
    allow(unused_variables)
)]
pub fn register_drivers(registry: &mut DriverRegistry) {
    #[cfg(feature = "cloudflare")]
    cloudflare::register_driver(registry);
    #[cfg(feature = "vercel")]
    vercel::register_driver(registry);
}
