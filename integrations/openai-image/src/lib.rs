//! OpenAI Image Generation integration.
//!
//! Provides the `gpt_image_gen` capability with `generate_image` and
//! `edit_image` tools, backed by OpenAI's GPT Image API. The capability
//! auto-registers through the inventory [`IntegrationPlugin`] system, so
//! linking this crate into a binary is enough to make it available.
//!
//! Part of the [Everruns](https://everruns.com) ecosystem.
//!
//! ```rust
//! use everruns_integrations_openai_image::GptImageGenCapability;
//! let _capability = GptImageGenCapability;
//! ```
//!
//! [`IntegrationPlugin`]: everruns_core::capabilities::IntegrationPlugin
//!
//! This capability lives outside the `everruns-openai` driver crate so that
//! provider crates stay pure `ChatDriver` implementations with no dependency
//! on core's capability/tool/session runtime.

mod image_capability;
mod images;

pub use image_capability::{EditImageTool, GenerateImageTool, GptImageGenCapability};
