//! Channel-specific plumbing that is not a domain record or an HTTP-agnostic
//! domain service. One child module per external channel.
//!
//! Decision: a channel's delivery, actions, provisioning and its inbound HTTP
//! handlers live together under its module, so the whole integration reads
//! from one tree. Domain records and storage for channels stay in `domains/`,
//! `records/` and `storage/`.

pub mod a2a;
pub mod ag_ui;
pub mod fcp;
pub mod public_chat;
pub mod slack;
pub mod voice;
