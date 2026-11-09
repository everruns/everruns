#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//! Code navigation for Everruns agents.
//!
//! Three capabilities that let an agent understand and change code by its
//! structure instead of by text search:
//!
//! - [`LspCapability`] (`lsp`): real language servers (rust-analyzer,
//!   typescript-language-server, pyright, gopls, clangd, or any you configure)
//!   for diagnostics, go-to-definition, references, hover, rename, symbols and
//!   code actions.
//! - [`AstGrepCapability`] and [`AstEditCapability`] (`ast_grep`, `ast_edit`):
//!   structural search and previewed rewrites with ast-grep patterns.
//! - [`RepoMapCapability`] (`repo_map`): compact symbol maps of a repository.
//!
//! The tools do not assume where the code is. Each capability is built over a
//! [`CodeHost`]:
//!
//! - [`SandboxCodeHost`] works on the session's sandbox. This is what a hosted
//!   Everruns deployment uses, and what [`CAPABILITY_PLUGINS`] registers.
//! - `LocalCodeHost` (feature `local`) works on a directory of this machine.
//!   This is what a terminal agent uses.
//!
//! # Example
//!
//! A capability over the session's sandbox exposes the seven `lsp_*` tools:
//!
//! ```
//! use everruns_coding::{LspCapability, SandboxCodeHost};
//! use everruns_contracts::runtime::capabilities::Capability;
//! use std::sync::Arc;
//!
//! let capability = LspCapability::new(Arc::new(SandboxCodeHost));
//! let names: Vec<String> = capability
//!     .tools()
//!     .iter()
//!     .map(|tool| tool.name().to_string())
//!     .collect();
//! assert!(names.contains(&"lsp_references".to_string()));
//! assert_eq!(names.len(), 7);
//! ```
//!
//! Part of the [Everruns](https://everruns.com) ecosystem.

mod ast_grep;
mod failure;
mod host;
mod lsp;
mod narration;
mod repo_map;
mod source;

pub use ast_grep::{
    AST_EDIT_CAPABILITY_ID, AST_GREP_CAPABILITY_ID, AstEditCapability, AstGrepCapability,
};
#[cfg(feature = "local")]
pub use host::LocalCodeHost;
pub use host::{CodeHost, FileOp, Pump, SandboxCodeHost, ServerCommand, ServerIo, SourceTree};
pub use lsp::{LSP_CAPABILITY_ID, LspCapability};
pub use repo_map::{REPO_MAP_CAPABILITY_ID, RepoMapCapability};
pub use source::SourceFilter;

use everruns_contracts::runtime::capabilities::IntegrationPlugin;
use std::sync::Arc;

/// Capability plugins this crate contributes to a hosted catalog. All of them
/// work on the session's sandbox.
pub const CAPABILITY_PLUGINS: &[IntegrationPlugin] = &[
    IntegrationPlugin {
        feature_flag: None,
        factory: || Box::new(LspCapability::hosted()),
    },
    IntegrationPlugin {
        feature_flag: None,
        factory: || Box::new(AstGrepCapability::new(Arc::new(SandboxCodeHost))),
    },
    IntegrationPlugin {
        feature_flag: None,
        factory: || Box::new(AstEditCapability::new(Arc::new(SandboxCodeHost))),
    },
    IntegrationPlugin {
        feature_flag: None,
        factory: || Box::new(RepoMapCapability::new(Arc::new(SandboxCodeHost))),
    },
];
