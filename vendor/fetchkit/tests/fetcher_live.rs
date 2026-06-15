//! Live integration tests for fetchers against real endpoints.
//!
//! Gated behind `--features live-tests` so they never run during normal `cargo test`.
//! Each test module maps 1:1 to a fetcher source file; CI runs only the modules
//! whose fetcher changed.
//!
//! Assertions are structural (field presence, non-empty content, expected substrings)
//! rather than exact-match, so tests tolerate minor upstream changes.
//!
//! Network errors (DNS, timeout, blocked) are treated as skips, not failures —
//! live tests should only fail on unexpected response structure, not infra issues.

#![cfg(feature = "live-tests")]

use fetchkit::{FetchError, FetchOptions, FetchRequest, FetchResponse, FetcherRegistry};

/// Shared options for live tests — default everything, both conversions on.
fn live_options() -> FetchOptions {
    FetchOptions {
        enable_markdown: true,
        enable_text: true,
        ..Default::default()
    }
}

fn registry() -> FetcherRegistry {
    FetcherRegistry::with_defaults()
}

/// Network errors that indicate infra problems, not fetcher bugs.
fn is_network_error(err: &FetchError) -> bool {
    matches!(
        err,
        FetchError::FirstByteTimeout
            | FetchError::BlockedUrl
            | FetchError::ConnectError(_)
            | FetchError::ClientBuildError(_)
            | FetchError::RequestError(_)
    )
}

/// Fetch and return Ok(response), or skip the test if the error is network-related.
async fn fetch_or_skip(url: &str) -> Option<FetchResponse> {
    let req = FetchRequest::new(url);
    match registry().fetch(req, live_options()).await {
        Ok(resp) => Some(resp),
        Err(e) if is_network_error(&e) => {
            eprintln!("SKIPPED (network): {url} — {e}");
            None
        }
        Err(e) => panic!("unexpected fetcher error for {url}: {e}"),
    }
}

/// Like fetch_or_skip but with as_markdown set.
async fn fetch_markdown_or_skip(url: &str) -> Option<FetchResponse> {
    let req = FetchRequest::new(url).as_markdown();
    match registry().fetch(req, live_options()).await {
        Ok(resp) => Some(resp),
        Err(e) if is_network_error(&e) => {
            eprintln!("SKIPPED (network): {url} — {e}");
            None
        }
        Err(e) => panic!("unexpected fetcher error for {url}: {e}"),
    }
}

// ---------------------------------------------------------------------------
// github_repo
// ---------------------------------------------------------------------------
mod live_github_repo {
    use super::*;

    #[tokio::test]
    async fn fetches_repo_metadata() {
        let Some(resp) = fetch_or_skip("https://github.com/rust-lang/rust").await else {
            return;
        };

        assert_eq!(resp.status_code, 200);
        let content = resp.content.expect("should have content");
        assert!(
            content.contains("rust-lang/rust") || content.to_lowercase().contains("rust"),
            "content should mention the repo"
        );
        assert!(!content.is_empty());
    }
}

// ---------------------------------------------------------------------------
// github_issue
// ---------------------------------------------------------------------------
mod live_github_issue {
    use super::*;

    #[tokio::test]
    async fn fetches_issue() {
        // Well-known issue: rust-lang/rust#1 (the very first issue)
        let Some(resp) = fetch_or_skip("https://github.com/rust-lang/rust/issues/1").await else {
            return;
        };

        assert_eq!(resp.status_code, 200);
        let content = resp.content.expect("should have content");
        assert!(!content.is_empty());
    }
}

// ---------------------------------------------------------------------------
// github_code
// ---------------------------------------------------------------------------
mod live_github_code {
    use super::*;

    #[tokio::test]
    async fn fetches_source_file() {
        let Some(resp) =
            fetch_or_skip("https://github.com/rust-lang/rust/blob/master/README.md").await
        else {
            return;
        };

        assert_eq!(resp.status_code, 200);
        let content = resp.content.expect("should have content");
        assert!(
            content.to_lowercase().contains("rust"),
            "README should mention Rust"
        );
    }
}

// ---------------------------------------------------------------------------
// twitter
// ---------------------------------------------------------------------------
mod live_twitter {
    use super::*;

    #[tokio::test]
    async fn fetches_tweet() {
        // Rust lang announcement tweet — stable, public
        let Some(resp) = fetch_or_skip("https://x.com/rustlang/status/1821986021505405014").await
        else {
            return;
        };

        // Twitter APIs are unreliable; accept any non-panic response as proof
        // the fetcher handled it. Only assert structure on 200.
        if resp.status_code == 200 {
            assert!(resp.content.is_some());
        }
    }
}

// ---------------------------------------------------------------------------
// stackoverflow
// ---------------------------------------------------------------------------
mod live_stackoverflow {
    use super::*;

    #[tokio::test]
    async fn fetches_question() {
        // "What is a NullPointerException" — one of the most famous SO questions
        let Some(resp) = fetch_or_skip(
            "https://stackoverflow.com/questions/218384/what-is-a-nullpointerexception-and-how-do-i-fix-it",
        ).await else {
            return;
        };

        assert_eq!(resp.status_code, 200);
        let content = resp.content.expect("should have content");
        assert!(
            content.to_lowercase().contains("null"),
            "content should mention null"
        );
    }
}

// ---------------------------------------------------------------------------
// package_registry
// ---------------------------------------------------------------------------
mod live_package_registry {
    use super::*;

    #[tokio::test]
    async fn fetches_crate() {
        let Some(resp) = fetch_or_skip("https://crates.io/crates/serde").await else {
            return;
        };

        assert_eq!(resp.status_code, 200);
        let content = resp.content.expect("should have content");
        assert!(
            content.to_lowercase().contains("serde"),
            "content should mention serde"
        );
    }

    #[tokio::test]
    async fn fetches_pypi_package() {
        let Some(resp) = fetch_or_skip("https://pypi.org/project/requests/").await else {
            return;
        };

        assert_eq!(resp.status_code, 200);
        let content = resp.content.expect("should have content");
        assert!(
            content.to_lowercase().contains("requests"),
            "content should mention requests"
        );
    }

    #[tokio::test]
    async fn fetches_npm_package() {
        let Some(resp) = fetch_or_skip("https://www.npmjs.com/package/express").await else {
            return;
        };

        assert_eq!(resp.status_code, 200);
        let content = resp.content.expect("should have content");
        assert!(
            content.to_lowercase().contains("express"),
            "content should mention express"
        );
    }
}

// ---------------------------------------------------------------------------
// wikipedia
// ---------------------------------------------------------------------------
mod live_wikipedia {
    use super::*;

    #[tokio::test]
    async fn fetches_article() {
        let Some(resp) =
            fetch_or_skip("https://en.wikipedia.org/wiki/Rust_(programming_language)").await
        else {
            return;
        };

        assert_eq!(resp.status_code, 200);
        let content = resp.content.expect("should have content");
        assert!(
            content.to_lowercase().contains("rust"),
            "article should mention Rust"
        );
    }
}

// ---------------------------------------------------------------------------
// youtube
// ---------------------------------------------------------------------------
mod live_youtube {
    use super::*;

    #[tokio::test]
    async fn fetches_video_metadata() {
        // "Me at the zoo" — first YouTube video ever, very stable
        let Some(resp) = fetch_or_skip("https://www.youtube.com/watch?v=jNQXAC9IVRw").await else {
            return;
        };

        assert_eq!(resp.status_code, 200);
        let content = resp.content.expect("should have content");
        assert!(!content.is_empty());
    }
}

// ---------------------------------------------------------------------------
// arxiv
// ---------------------------------------------------------------------------
mod live_arxiv {
    use super::*;

    #[tokio::test]
    async fn fetches_paper() {
        // "Attention Is All You Need"
        let Some(resp) = fetch_or_skip("https://arxiv.org/abs/1706.03762").await else {
            return;
        };

        assert_eq!(resp.status_code, 200);
        let content = resp.content.expect("should have content");
        assert!(
            content.to_lowercase().contains("attention"),
            "paper should mention attention"
        );
    }
}

// ---------------------------------------------------------------------------
// hackernews
// ---------------------------------------------------------------------------
mod live_hackernews {
    use super::*;

    #[tokio::test]
    async fn fetches_story() {
        // HN item 1 — the very first story
        let Some(resp) = fetch_or_skip("https://news.ycombinator.com/item?id=1").await else {
            return;
        };

        assert_eq!(resp.status_code, 200);
        let content = resp.content.expect("should have content");
        assert!(!content.is_empty());
    }
}

// ---------------------------------------------------------------------------
// rss_feed
// ---------------------------------------------------------------------------
mod live_rss_feed {
    use super::*;

    #[tokio::test]
    async fn fetches_rss() {
        // Rust blog RSS feed
        let Some(resp) = fetch_or_skip("https://blog.rust-lang.org/feed.xml").await else {
            return;
        };

        assert_eq!(resp.status_code, 200);
        let content = resp.content.expect("should have content");
        assert!(
            content.to_lowercase().contains("rust"),
            "Rust blog feed should mention Rust"
        );
    }
}

// ---------------------------------------------------------------------------
// docs_site
// ---------------------------------------------------------------------------
mod live_docs_site {
    use super::*;

    #[tokio::test]
    async fn fetches_docs_rs() {
        let Some(resp) = fetch_or_skip("https://docs.rs/serde/latest/serde/").await else {
            return;
        };

        assert_eq!(resp.status_code, 200);
        let content = resp.content.expect("should have content");
        assert!(
            content.to_lowercase().contains("serde"),
            "docs.rs page should mention serde"
        );
    }
}

// ---------------------------------------------------------------------------
// default (generic HTTP)
// ---------------------------------------------------------------------------
mod live_default {
    use super::*;

    #[tokio::test]
    async fn fetches_plain_html() {
        let Some(resp) = fetch_markdown_or_skip("https://example.com").await else {
            return;
        };

        assert_eq!(resp.status_code, 200);
        let content = resp.content.expect("should have content");
        assert!(
            content.contains("Example Domain"),
            "example.com should contain 'Example Domain'"
        );
        assert_eq!(resp.format, Some("markdown".to_string()));
    }
}
