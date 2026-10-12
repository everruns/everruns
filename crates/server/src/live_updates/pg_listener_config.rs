// PostgreSQL LISTEN/NOTIFY connection selection.
// Decision: Keep pooled DATABASE_URL for ordinary queries, but route session-scoped
// LISTEN/NOTIFY traffic through a direct URL when provided.
// Decision: Reject obvious pooler/proxy URLs for LISTEN/NOTIFY because pooled
// endpoints can break sqlx listener semantics and surface NotificationResponse
// frames on unrelated query traffic.

use anyhow::{Result, bail};
use url::Url;

const DATABASE_URL_ENV: &str = "DATABASE_URL";
const DATABASE_UNPOOLED_URL_ENV: &str = "DATABASE_UNPOOLED_URL";

pub fn resolve_pg_listener_database_url(
    database_url: &str,
    database_unpooled_url: Option<&str>,
) -> Result<String> {
    let trimmed_unpooled = database_unpooled_url
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let listener_url = trimmed_unpooled.unwrap_or(database_url);
    let source = if trimmed_unpooled.is_some() {
        DATABASE_UNPOOLED_URL_ENV
    } else {
        DATABASE_URL_ENV
    };

    if looks_like_pooled_pg_endpoint(listener_url) {
        bail!(
            "{source} points to a pooled/proxied PostgreSQL endpoint. \
             PG LISTEN/NOTIFY requires a direct session-scoped connection. \
             Set {DATABASE_UNPOOLED_URL_ENV} to a direct PostgreSQL URL for listener paths."
        );
    }

    Ok(listener_url.to_string())
}

fn looks_like_pooled_pg_endpoint(url: &str) -> bool {
    let Ok(parsed) = Url::parse(url) else {
        return false;
    };

    let host = parsed.host_str().map(str::to_ascii_lowercase);
    let host_looks_pooled = host.as_deref().is_some_and(|host| {
        host.contains("-pooler.") || host.contains(".pooler.") || host.contains("pgbouncer")
    });
    let query_requests_pooling = parsed
        .query_pairs()
        .any(|(key, _)| key.eq_ignore_ascii_case("pool_mode"));

    host_looks_pooled || query_requests_pooling
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uses_direct_listener_url_when_present() {
        let listener_url = resolve_pg_listener_database_url(
            "postgres://app@ep-foo-pooler.us-east-1.aws.neon.tech/db",
            Some("postgres://app@ep-foo.us-east-1.aws.neon.tech/db"),
        )
        .unwrap();

        assert_eq!(
            listener_url,
            "postgres://app@ep-foo.us-east-1.aws.neon.tech/db"
        );
    }

    #[test]
    fn allows_direct_database_url_without_override() {
        let listener_url = resolve_pg_listener_database_url(
            "postgres://postgres:postgres@localhost:5432/everruns",
            None,
        )
        .unwrap();

        assert_eq!(
            listener_url,
            "postgres://postgres:postgres@localhost:5432/everruns"
        );
    }

    #[test]
    fn rejects_pooled_database_url_for_listeners() {
        let error = resolve_pg_listener_database_url(
            "postgres://app@ep-foo-pooler.us-east-1.aws.neon.tech/db",
            None,
        )
        .unwrap_err();

        assert!(
            error
                .to_string()
                .contains("PG LISTEN/NOTIFY requires a direct session-scoped connection")
        );
    }

    #[test]
    fn rejects_pooled_unpooled_override() {
        let error = resolve_pg_listener_database_url(
            "postgres://postgres:postgres@localhost:5432/everruns",
            Some("postgres://app@ep-foo-pooler.us-east-1.aws.neon.tech/db"),
        )
        .unwrap_err();

        assert!(error.to_string().contains(DATABASE_UNPOOLED_URL_ENV));
    }

    #[test]
    fn ignores_passwords_that_happen_to_mention_poolers() {
        let listener_url = resolve_pg_listener_database_url(
            "postgres://app:pgbouncer-secret@localhost:5432/everruns",
            None,
        )
        .unwrap();

        assert_eq!(
            listener_url,
            "postgres://app:pgbouncer-secret@localhost:5432/everruns"
        );
    }

    #[test]
    fn rejects_explicit_pool_mode_query_params() {
        let error = resolve_pg_listener_database_url(
            "postgres://app@localhost:5432/everruns?pool_mode=session",
            None,
        )
        .unwrap_err();

        assert!(
            error
                .to_string()
                .contains("PG LISTEN/NOTIFY requires a direct session-scoped connection")
        );
    }
}
