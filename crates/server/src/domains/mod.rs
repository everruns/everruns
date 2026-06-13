// Feature-oriented domain modules.
//
// Each domain owns its commands (user-facing operations), queries (shared
// read/write helpers), and types. See specs/domains.md for the full pattern.

pub mod common;

pub mod agent_identities;
pub mod agents;
pub mod apps;
pub mod audit_logs;
pub mod budgets;
pub mod capabilities;
pub mod evals;
pub mod events;
pub mod harnesses;
pub mod images;
pub mod knowledge_bases;
pub mod models;
pub mod providers;
pub mod mcp_servers;
pub mod memory;
pub mod messages;
pub mod notifications;
pub mod org_resolver;
pub mod organizations;
pub mod payments;
pub mod plugins;
pub mod reporting;
pub mod schedules;
pub mod session_commands;
pub mod session_databases;
pub mod session_files;
pub mod session_git;
pub mod session_resources;
pub mod session_sandbox;
pub mod session_schedules;
pub mod session_storage;
pub mod session_tasks;
pub mod sessions;
pub mod skills;
pub mod system;
pub mod tool_results;
pub mod users;
pub mod workspaces;
