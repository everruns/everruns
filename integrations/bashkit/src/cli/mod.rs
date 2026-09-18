// The `everruns` noun-verb command tree.
//
// One grammar, several hosts. A command's canonical identity stays its flat
// wire name (`list_agents`); this adds the spelling a caller types
// (`everruns agents list`) and the bounded help that makes such a surface
// discoverable at all.
//
// The tree is deliberately transport-neutral. Two adapters exist because two
// hosts differ in what they can see, not because the grammar differs:
//
//   * A plain bash tool hands a builtin its raw argv, so [`CliBuiltin`] parses
//     the tree directly.
//   * A `ScriptedTool` host parses `--flag` pairs before a builtin runs and
//     never surfaces bare words, so [`rewrite`] converts the tree spelling
//     into the flat name at statement boundaries before the interpreter sees
//     it. Everything downstream — schema, coercion, authorization, error
//     handling — is then the same code the flat name already went through.
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
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::sync::{Mutex, OnceLock};

use async_trait::async_trait;
use bashkit::ExecResult;
use serde_json::Value;

mod builtin;
mod forwarding;
mod help;
mod rewrite;
mod tree;

pub use builtin::{CliBuiltin, CliCommandSourceHandle, EverrunsBuiltin};
pub use forwarding::{ForwardingBuiltin, forwarded_command_line, forwarding_builtin_for};
pub use help::render_help;
pub use rewrite::rewrite;
pub use tree::{CliCommandSource, CliCommandSpec, CliTree, Leaf};

/// Builtin the rewriter emits for any help or usage request.
pub const HELP_BUILTIN: &str = "everruns_help";

/// Root token that introduces a tree invocation.
pub const ROOT: &str = "everruns";
