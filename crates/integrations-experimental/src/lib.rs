#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
#![doc = include_str!("../README.md")]

#[cfg(feature = "agentid")]
pub mod agentid;
#[cfg(feature = "sprites")]
pub mod sprites;
