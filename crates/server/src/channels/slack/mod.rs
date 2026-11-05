//! Slack channel: everything that talks to Slack, in one tree.
//!
//! - [`events`]: inbound HTTP handlers (Events API, interactivity, manifest)
//!   and their router, mounted without auth.
//! - [`install`]: one-click OAuth install handlers and router.
//! - [`delivery`]: posting replies, streams, status and titles to Slack.
//! - [`host`]: Slack as a driver of the shared channel host.
//! - [`scheduled`]: Slack conversations a schedule trigger runs in.
//! - [`actions`]: Slack actions an agent invokes through the worker seam.
//! - [`approvals`]: approval cards and their button decisions.
//! - [`provisioning`]: creating and configuring Slack apps for endpoints.
//! - [`api`] / [`api_error`]: shared Slack Web API client and its errors.
//!
//! Slack-shaped domain records stay in `records/` (`slack_channel`,
//! `slack_provisioning`), domain rules in `domains/agent_channels/`, and the
//! org connection store in `storage/`.

pub mod actions;
pub mod approvals;
pub(crate) mod code;
pub mod delivery;
pub mod events;
pub(crate) mod host;
#[cfg(test)]
pub(crate) mod host_tests;
pub mod install;
mod manager_connect;
pub mod provisioning;
pub(crate) mod scheduled;
