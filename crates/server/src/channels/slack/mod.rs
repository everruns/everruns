//! Slack channel: everything that talks to Slack, in one tree.
//!
//! - [`events`]: inbound HTTP handlers (Events API, interactivity, manifest)
//!   and their router, mounted without auth.
//! - [`install`]: one-click OAuth install handlers and router.
//! - [`delivery`]: event-driven posting of agent output back to Slack.
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
pub mod delivery;
pub mod events;
pub mod install;
pub mod provisioning;
