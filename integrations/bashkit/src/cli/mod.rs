// The `everruns` noun-verb command tree.
//
// One grammar, several hosts. A command's canonical identity stays its flat
// wire name (`list_agents`); this adds the spelling a caller types
// (`everruns agents list`) and the bounded help that makes such a surface
// discoverable at all.
//
// Grammar, help and argv resolution live in `everruns-cli-contract`
// ([`CliTree`] is its `CommandTree`), so this crate, the server's MCP surface
// and the terminal CLI resolve a line identically. What this module adds is
// the bash side: [`CliBuiltin`] hands a builtin's raw argv to the tree, and
// [`ForwardingBuiltin`] carries a line to a tool for a host that cannot link
// the commands in.
//
// Where the commands come from is the host's business: [`CliCommandSource`]
// is the seam. A server backs it with its domain-command catalog; a Framework
// application backs it with whatever it owns. A host that provides no source
// gets no tree and advertises none, rather than promising a surface it cannot
// serve.
//
// Help is bounded by the tree's shape rather than by a cap: the root lists
// nouns, a node lists its children, a leaf renders its own flags. That is what
// makes `--help` affordable where a flat namespace of hundreds of commands has
// to forbid it.
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::{Mutex, OnceLock};

use async_trait::async_trait;
use bashkit::ExecResult;
use serde_json::Value;

mod builtin;
mod forwarding;
mod tree;

pub use builtin::{CliBuiltin, CliCommandSourceHandle, EverrunsBuiltin};
pub use everruns_cli_contract::tree::{CommandTree as CliTree, Leaf, ROOT};
pub use forwarding::{ForwardingBuiltin, forwarded_command_line, forwarding_builtin_for};
pub use tree::{CliCommandSource, CliCommandSpec, tree_from_source};
