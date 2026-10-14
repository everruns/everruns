//! A shared, temporary PostgreSQL cluster for dev mode and tests.
//!
//! `postgresql_embedded` owns binary installation, initialization, startup, and
//! shutdown. This adapter adds process-wide sharing and template database copies.
//! Embedded callers must run as a non-root user, like the standard CI runners.
//!
//! The shared value is static, so Rust never drops it at process exit. A small
//! pipe-watching helper stops the cluster even if its owner is killed; otherwise
//! the library's normal Drop cleanup would leave every test binary's server alive.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

use std::collections::HashMap;
use std::fs;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use postgresql_embedded::{PostgreSQL, Settings, VersionReq};
use sqlx::Connection;
use tempfile::TempDir;
use tokio::sync::OnceCell;

/// PostgreSQL major version used by dev mode and tests.
pub const MAJOR_VERSION: u64 = 17;

static SHARED: OnceCell<EmbeddedPostgres> = OnceCell::const_new();

/// A real PostgreSQL server with disposable data and durability disabled.
pub struct EmbeddedPostgres {
    // Fields drop in declaration order: the library stops PostgreSQL before the
    // helper removes the enclosing private directory.
    postgres: PostgreSQL,
    _cleanup: OwnerCleanup,
}

impl EmbeddedPostgres {
    /// Start one cluster on first use and share it for the process's lifetime.
    pub async fn shared() -> Result<&'static Self> {
        SHARED.get_or_try_init(Self::start).await
    }

    /// Connection URL for `database` on this cluster.
    pub fn url(&self, database: &str) -> String {
        self.postgres.settings().url(database)
    }

    /// Create an empty database or a private copy of a disconnected template.
    pub async fn create_database(&self, name: &str, template: Option<&str>) -> Result<()> {
        let mut sql = format!("CREATE DATABASE {}", identifier(name)?);
        if let Some(template) = template {
            // Disposable clusters avoid WAL amplification when copying the
            // migrated schema for hundreds of isolated tests.
            sql.push_str(&format!(
                " TEMPLATE {} STRATEGY FILE_COPY",
                identifier(template)?
            ));
        }
        let mut conn = sqlx::PgConnection::connect(&self.url("postgres")).await?;
        sqlx::raw_sql(sqlx::AssertSqlSafe(sql))
            .execute(&mut conn)
            .await
            .with_context(|| format!("creating database {name}"))?;
        conn.close().await?;
        Ok(())
    }

    /// Drop a database, disconnecting any remaining test connections.
    pub async fn drop_database(&self, name: &str) -> Result<()> {
        let sql = format!("DROP DATABASE IF EXISTS {} WITH (FORCE)", identifier(name)?);
        let mut conn = sqlx::PgConnection::connect(&self.url("postgres")).await?;
        sqlx::raw_sql(sqlx::AssertSqlSafe(sql))
            .execute(&mut conn)
            .await
            .with_context(|| format!("dropping database {name}"))?;
        conn.close().await?;
        Ok(())
    }

    async fn start() -> Result<Self> {
        // PostgreSQL itself refuses root. Keep the library's lifecycle intact
        // instead of maintaining a second implementation for privileged callers.
        // SAFETY: geteuid has no preconditions.
        if unsafe { libc::geteuid() } == 0 {
            bail!(
                "embedded PostgreSQL requires a non-root user; run DEV_MODE/tests as a \
                 non-root user, or use external PostgreSQL with DEV_MODE=false"
            );
        }
        // A short prefix keeps Unix socket paths within macOS's length limit.
        let root = tempfile::Builder::new().prefix("evr-pg-").tempdir()?;
        let settings = Settings {
            // Use the same cache and archive source as existing installations.
            releases_url: "https://github.com/zonkyio/embedded-postgres-binaries".into(),
            version: VersionReq::parse(&format!("^{MAJOR_VERSION}"))?,
            installation_dir: cache_dir()?,
            password_file: root.path().join("pwfile"),
            data_dir: root.path().join("data"),
            host: "127.0.0.1".into(),
            port: 0,
            username: "postgres".into(),
            password: uuid::Uuid::new_v4().simple().to_string(),
            temporary: true,
            timeout: Some(Duration::from_secs(60)),
            configuration: HashMap::from([
                ("listen_addresses".into(), "127.0.0.1".into()),
                ("fsync".into(), "off".into()),
                ("synchronous_commit".into(), "off".into()),
                ("full_page_writes".into(), "off".into()),
                ("max_connections".into(), "500".into()),
                ("wal_level".into(), "minimal".into()),
                ("max_wal_senders".into(), "0".into()),
                ("max_wal_size".into(), "128MB".into()),
                ("min_wal_size".into(), "32MB".into()),
            ]),
            trust_installation_dir: false,
            socket_dir: Some(root.path().to_owned()),
        };
        let mut postgres = PostgreSQL::new(settings);
        postgres
            .setup()
            .await
            .context("set up embedded PostgreSQL")?;
        // The upstream initializer uses password authentication; retain SCRAM
        // for the private cluster instead of sending its password in cleartext.
        fs::write(
            postgres.settings().data_dir.join("pg_hba.conf"),
            "local all all scram-sha-256\nhost all all 127.0.0.1/32 scram-sha-256\n",
        )?;
        postgres
            .start()
            .await
            .context("start embedded PostgreSQL")?;
        let cleanup = OwnerCleanup::start(&postgres, &root)?;
        // Keep the directory owned locally until the watcher starts. On an
        // error, PostgreSQL must stop before TempDir removes its live data.
        let _ = root.keep();
        tracing::info!(
            port = postgres.settings().port,
            "embedded PostgreSQL started"
        );
        Ok(Self {
            postgres,
            _cleanup: cleanup,
        })
    }
}

/// The pipe closes on normal exit, abort, or SIGKILL, without polling the owner.
struct OwnerCleanup {
    input: Option<ChildStdin>,
    child: Child,
}

impl OwnerCleanup {
    fn start(postgres: &PostgreSQL, root: &TempDir) -> Result<Self> {
        // The library owns pg_ctl and its startup/stop protocol. This helper
        // only covers owner termination, which Drop cannot observe. Paths are
        // positional arguments, never interpolated into shell source. Remove
        // data only after shutdown, or after the library already removed it.
        let mut child = Command::new("/bin/sh")
            // Terminal Ctrl+C and process-group termination must not kill the
            // watcher alongside its owner before it can stop PostgreSQL.
            .process_group(0)
            .args([
                "-c",
                r#"read -r _ || :
if "$1" -D "$2/data" stop -m fast -w -t 10 >/dev/null 2>&1 || [ ! -e "$2/data/postmaster.pid" ]; then
    /bin/rm -rf -- "$2"
fi"#,
                "embedded-postgres-cleanup",
            ])
            .arg(postgres.settings().binary_dir().join("pg_ctl"))
            .arg(root.path())
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .context("start embedded PostgreSQL owner cleanup")?;
        let input = child.stdin.take().context("open owner cleanup pipe")?;
        Ok(Self {
            input: Some(input),
            child,
        })
    }
}

impl Drop for OwnerCleanup {
    fn drop(&mut self) {
        drop(self.input.take());
        let _ = self.child.wait();
    }
}

fn identifier(name: &str) -> Result<String> {
    if name.is_empty()
        || !name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
    {
        bail!("database name must be lowercase letters, digits and underscores: {name:?}");
    }
    Ok(format!("\"{name}\""))
}

fn cache_dir() -> Result<PathBuf> {
    if let Some(dir) = std::env::var_os("EVERRUNS_PG_CACHE") {
        return Ok(PathBuf::from(dir));
    }
    if let Some(xdg) = std::env::var_os("XDG_CACHE_HOME") {
        return Ok(PathBuf::from(xdg).join("everruns/postgres"));
    }
    let home = std::env::var_os("HOME").context("HOME is not set")?;
    Ok(PathBuf::from(home).join(".cache/everruns/postgres"))
}

#[cfg(test)]
mod tests;
