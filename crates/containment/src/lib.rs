#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
#![deny(missing_docs)]

//! Kernel containment for commands an agent runs on the machine it is running on.
//!
//! `everruns-host` answers *where* a command runs ([`Compute`]); this package
//! answers *what that command may touch*. It is the implementation behind
//! `ContainmentLevel::Native`: Seatbelt on macOS, Landlock plus seccomp on
//! Linux, ported from Yolop's `src/exec/sandbox.rs`.
//!
//! Two invariants carry over from Yolop and are the reason this is a boundary
//! rather than a helper:
//!
//! 1. **Model input never selects a host executable or widens a mount.** A
//!    caller configures [`SandboxOptions`] once; a tool passes only a script.
//! 2. **It fails closed.** When the OS primitive a mode needs is unavailable,
//!    [`SandboxProvider::command`] returns an error instead of degrading to an
//!    uncontained host process. The one documented exception is Windows, which
//!    has no implementation and says so through [`danger_warning`] at every
//!    mode.
//!
//! This crate is part of the [Everruns](https://everruns.com) ecosystem.
//!
//! # Example
//!
//! ```
//! use everruns_containment::{ContainmentMode, SandboxOptions, provider};
//!
//! let contained = provider(SandboxOptions::new(ContainmentMode::WorkspaceWrite));
//! assert_eq!(contained.mode(), ContainmentMode::WorkspaceWrite);
//! assert_eq!(contained.mode().as_str(), "workspace-write");
//!
//! // Full access is the honest name for no containment at all.
//! let open = provider(SandboxOptions::new(ContainmentMode::FullAccess));
//! assert!(everruns_containment::danger_warning(open.mode()).is_some());
//! ```
//!
//! [`Compute`]: https://docs.rs/everruns-host

pub mod policy;
pub mod worker;

mod sandbox;

pub use sandbox::{
    ContainmentMode, SandboxLauncher, SandboxOptions, SandboxProvider, configure_stdio,
    danger_warning, network_access, provider,
};
