#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//! Live smoke tests for gateway model discovery.
//!
//! These exercise the real `list_models` path — HTTP fetch, deserialization,
//! filtering, and capability profiling — against each gateway's own API. The
//! wire tests in `chat_wire.rs` pin the request shape against wiremock; only a
//! live run can catch a vendor changing the response.
//!
//! Ignored by default (network + credentials); run manually:
//!
//! ```text
//! doppler run -- cargo test -p everruns-drivers --test discovery_live --all-features -- --ignored --nocapture
//! ```

#[cfg(feature = "vercel")]
mod vercel_discovery {
    use everruns_drivers::vercel;

    /// Vercel serves a full catalog, so discovery must return a non-empty list
    /// of namespaced ids.
    #[tokio::test]
    #[ignore = "live network + AI_GATEWAY_API_KEY"]
    async fn vercel_discovers_namespaced_models() {
        let api_key = std::env::var("AI_GATEWAY_API_KEY")
            .expect("AI_GATEWAY_API_KEY must be set for the live smoke test");

        let models = vercel::provider("vercel", api_key)
            .list_models()
            .await
            .expect("list_models should succeed")
            .expect("Vercel should support model listing");

        assert!(!models.is_empty(), "expected a non-empty model catalog");

        // Ids are `vendor/model`; the gateway routes on that prefix, so an id
        // without one would not reach any upstream.
        let namespaced = models.iter().filter(|m| m.model_id.contains('/')).count();
        assert_eq!(
            namespaced,
            models.len(),
            "every Vercel model id should be namespaced vendor/model"
        );

        eprintln!("Vercel: {} models", models.len());
        for m in models.iter().take(5) {
            eprintln!("  {} (owned_by={:?})", m.model_id, m.owned_by);
        }
    }

    /// A base URL that is not Vercel's own host must not be probed: discovery
    /// declines rather than sending the account's bearer token to it.
    #[tokio::test]
    #[ignore = "live network + AI_GATEWAY_API_KEY"]
    async fn vercel_discovery_declines_a_foreign_host() {
        let api_key = std::env::var("AI_GATEWAY_API_KEY")
            .expect("AI_GATEWAY_API_KEY must be set for the live smoke test");

        let models = vercel::provider("vercel", api_key)
            .base_url("https://example.invalid/v1")
            .list_models()
            .await
            .expect("declining is not an error");

        assert!(
            models.is_none(),
            "discovery must not probe a non-Vercel host"
        );
    }
}

#[cfg(feature = "cloudflare")]
mod cloudflare_discovery {
    use everruns_drivers::cloudflare;

    fn credentials() -> (String, String) {
        (
            std::env::var("CLOUDFLARE_ACCOUNT_ID")
                .expect("CLOUDFLARE_ACCOUNT_ID must be set for the live smoke test"),
            std::env::var("CLOUDFLARE_API_TOKEN")
                .expect("CLOUDFLARE_API_TOKEN must be set for the live smoke test"),
        )
    }

    /// Cloudflare lists the Workers AI half of the catalog. Every id must be a
    /// `@cf/` one, and the text-generation filter must have excluded the
    /// embedding/speech/image models the same endpoint returns.
    #[tokio::test]
    #[ignore = "live network + CLOUDFLARE_API_TOKEN + CLOUDFLARE_ACCOUNT_ID"]
    async fn cloudflare_discovers_workers_ai_chat_models() {
        let (account_id, api_token) = credentials();

        let models = cloudflare::provider("cloudflare", &account_id, api_token, None)
            .list_models()
            .await
            .expect("list_models should succeed")
            .expect("Cloudflare should support model listing");

        assert!(!models.is_empty(), "expected a non-empty model catalog");
        for m in &models {
            assert!(
                m.model_id.starts_with("@cf/"),
                "only Workers AI ids are listed, got {}",
                m.model_id
            );
        }

        // A profile carries what the catalog advertises; at least one model
        // must report a context window, or the mapping has silently stopped
        // reading the properties list.
        let with_context = models
            .iter()
            .filter_map(|m| m.discovered_profile.as_ref())
            .filter(|p| p.limits.is_some())
            .count();
        assert!(
            with_context > 0,
            "expected at least one model to advertise a context window"
        );

        eprintln!("Cloudflare: {} chat models", models.len());
        for m in models.iter().take(5) {
            let ctx = m
                .discovered_profile
                .as_ref()
                .and_then(|p| p.limits.as_ref())
                .map(|l| l.context);
            let tools = m.discovered_profile.as_ref().is_some_and(|p| p.tool_call);
            eprintln!("  {} ctx={ctx:?} tools={tools}", m.model_id);
        }
    }

    /// Same host gate as Vercel: a proxy base URL is never probed.
    #[tokio::test]
    #[ignore = "live network + CLOUDFLARE_API_TOKEN + CLOUDFLARE_ACCOUNT_ID"]
    async fn cloudflare_discovery_declines_a_foreign_host() {
        let (account_id, api_token) = credentials();

        let models = cloudflare::provider("cloudflare", &account_id, api_token, None)
            .base_url("https://example.invalid/ai/v1")
            .list_models()
            .await
            .expect("declining is not an error");

        assert!(
            models.is_none(),
            "discovery must not probe a non-Cloudflare host"
        );
    }
}
