//! FetchKit - AI-friendly web content fetching library
//!
//! This crate provides a reusable library API for fetching web content,
//! with optional HTML to markdown/text conversion optimized for LLM consumption.
//!
//! # Quick Start
//!
//! ```no_run
//! use fetchkit::{FetchRequest, fetch};
//!
//! # async fn example() -> Result<(), fetchkit::FetchError> {
//! let request = FetchRequest::new("https://example.com").as_markdown();
//! let response = fetch(request).await?;
//! println!("Content: {}", response.content.unwrap_or_default());
//! # Ok(())
//! # }
//! ```
//!
//! # Tool Builder
//!
//! For more control, use the [`ToolBuilder`] to configure options:
//!
//! ```no_run
//! use fetchkit::{FetchRequest, ToolBuilder};
//!
//! # async fn example() -> Result<(), fetchkit::FetchError> {
//! let tool = ToolBuilder::new()
//!     .enable_markdown(true)
//!     .user_agent("MyBot/1.0")
//!     .block_prefix("https://blocked.example.com")
//!     .build();
//!
//! let request = FetchRequest::new("https://example.com");
//! let response = tool.execute(request).await?;
//! # Ok(())
//! # }
//! ```
//!
//! # HTML Conversion
//!
//! Convert HTML to markdown or plain text directly:
//!
//! ```
//! use fetchkit::{html_to_markdown, html_to_text};
//!
//! let html = "<h1>Hello</h1><p>World</p>";
//! let md = html_to_markdown(html);
//! assert!(md.contains("# Hello"));
//!
//! let text = html_to_text(html);
//! assert!(text.contains("Hello"));
//! ```
//!
//! # Fetcher System
//!
//! FetchKit uses a pluggable fetcher system where specialized fetchers
//! handle specific URL patterns. The [`FetcherRegistry`] dispatches
//! requests to the appropriate fetcher based on URL matching.
//!
//! Built-in fetchers:
//! - [`ArXivFetcher`] - arXiv paper metadata and abstract
//! - [`DefaultFetcher`] - General HTTP/HTTPS fetcher with HTML conversion
//! - [`DocsSiteFetcher`] - llms.txt probe with DefaultFetcher fallback
//! - [`GitHubCodeFetcher`] - GitHub source file content with language metadata
//! - [`HackerNewsFetcher`] - Hacker News thread content via Firebase API
//! - [`GitHubIssueFetcher`] - GitHub issue and PR metadata with comments
//! - [`GitHubRepoFetcher`] - GitHub repository metadata and README
//! - [`PackageRegistryFetcher`] - PyPI, crates.io, npm package metadata
//! - [`RSSFeedFetcher`] - RSS/Atom feed parsing
//! - [`StackOverflowFetcher`] - Stack Overflow Q&A content
//! - [`TwitterFetcher`] - Twitter/X tweet content with article metadata
//! - [`WikipediaFetcher`] - Wikipedia article content via MediaWiki API
//! - [`YouTubeFetcher`] - YouTube video metadata via oEmbed

#[cfg(feature = "bot-auth")]
pub mod bot_auth;

pub mod client;
mod convert;
mod dns;
mod error;
pub mod fetchers;
pub mod file_saver;
mod tool;
pub mod transport;
mod types;

pub use client::{batch_fetch, batch_fetch_with_options, fetch, fetch_with_options, FetchOptions};
pub use convert::{
    extract_headings, extract_metadata, html_to_markdown, html_to_text, strip_boilerplate,
};
pub use dns::DnsPolicy;
pub use error::{FetchError, ToolError};
pub use fetchers::{
    ArXivFetcher, DefaultFetcher, DocsSiteFetcher, Fetcher, FetcherRegistry, GitHubCodeFetcher,
    GitHubIssueFetcher, GitHubRepoFetcher, HackerNewsFetcher, PackageRegistryFetcher,
    RSSFeedFetcher, StackOverflowFetcher, TwitterFetcher, WikipediaFetcher, YouTubeFetcher,
};
pub use file_saver::{FileSaveError, FileSaver, LocalFileSaver, SaveResult};
pub use tool::{
    Tool, ToolBuilder, ToolExecution, ToolImage, ToolOutput, ToolOutputMetadata, ToolService,
    ToolStatus,
};
pub use transport::{
    BodyStream, HttpTransport, ReqwestTransport, TransportError, TransportMethod, TransportRequest,
    TransportResponse,
};
pub use types::{FetchRequest, FetchResponse, HttpMethod, PageLink, PageMetadata};

#[cfg(feature = "bot-auth")]
pub use bot_auth::{BotAuthConfig, BotAuthError};

/// Default User-Agent string
pub const DEFAULT_USER_AGENT: &str = "Everruns FetchKit/1.0";

/// Backward-compatible full description string with file-saving enabled.
pub const TOOL_DESCRIPTION: &str =
    "Fetch URL content as text or markdown; return metadata for binary responses or save bytes to file.";

/// Backward-compatible help document with file-saving enabled.
pub static TOOL_LLMTXT: std::sync::LazyLock<String> =
    std::sync::LazyLock::new(|| Tool::builder().enable_save_to_file(true).build().help());
