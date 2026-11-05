#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//! Image generation integration.
//!
//! Provides the `image_generation` capability with `generate_image` and
//! `edit_image` tools: stills, animated GIFs and SVGs from GPT Image, Muse
//! Image and Gemini image models, through OpenAI, Azure OpenAI, OpenRouter
//! and Meta providers. Hosts add it from [`CAPABILITY_PLUGINS`].
//!
//! Part of the [Everruns](https://everruns.com) ecosystem.
//!
//! ```rust
//! use everruns_contracts::runtime::capabilities::Capability;
//! use everruns_integrations::image_generation::ImageGenerationCapability;
//!
//! assert_eq!(ImageGenerationCapability::new().id(), "image_generation");
//! ```
//!
//! Requests go through the typed images driver in `everruns-drivers`, sent
//! over the host egress service so provider keys only reach validated
//! origins.

mod capability;
mod outputs;
mod resolve;
mod tools;

pub use capability::{CAPABILITY_PLUGINS, ImageGenerationCapability};
pub use resolve::PLATFORM_DEFAULT_IMAGE_MODEL;
pub use tools::{EditImageTool, GenerateImageTool};

#[cfg(test)]
mod capability_tests;
#[cfg(test)]
mod egress_tests;
#[cfg(test)]
mod narration_tests;
#[cfg(test)]
mod test_egress;
