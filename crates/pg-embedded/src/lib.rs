//! A temporary PostgreSQL cluster for the current process.
//!
//! [`EmbeddedPostgres::shared`] starts one cluster per process on first use
//! and returns it to every later caller. The cluster lives in a fresh data
//! directory under the system temp dir, listens on a free loopback port with a
//! random password, and runs with durability off: it is for dev runs and tests,
//! never for data that has to survive.
//!
//! Decision: the cluster is tied to the process, not to a value. On Linux the
//! server gets `PR_SET_PDEATHSIG`, so it dies with the process however the
//! process ends, and the next start in any process sweeps data directories
//! whose owner is gone. A guard with `Drop` would leak the server on a panic,
//! a Ctrl+C or a test harness that never drops statics.
//!
//! Decision: `initdb` refuses to run as root, and dev containers and CI often
//! are root. As root the cluster runs as `nobody` (uid 65534) and the binary
//! cache moves to the temp dir so that user can read it.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use postgresql_archive::{Version, VersionReq};
use sqlx::Connection;
use tokio::sync::OnceCell;

/// PostgreSQL major version, the one CI and production run.
pub const MAJOR_VERSION: u64 = 17;

const SUPERUSER: &str = "everruns";
const NOBODY: u32 = 65534;
const READY_TIMEOUT: Duration = Duration::from_secs(60);

static SHARED: OnceCell<EmbeddedPostgres> = OnceCell::const_new();

/// A running temporary cluster. See the module docs.
#[derive(Debug)]
pub struct EmbeddedPostgres {
    port: u16,
    password: String,
    dir: PathBuf,
}

impl EmbeddedPostgres {
    /// The cluster for this process, started on first call.
    ///
    /// The first call downloads the binaries when they are not cached yet
    /// (one ~30 MB archive per PostgreSQL release), then takes about a second.
    pub async fn shared() -> Result<&'static EmbeddedPostgres> {
        SHARED.get_or_try_init(Self::start).await
    }

    /// Connection URL for `database` on this cluster.
    pub fn url(&self, database: &str) -> String {
        format!(
            "postgres://{SUPERUSER}:{}@127.0.0.1:{}/{database}",
            self.password, self.port
        )
    }

    /// Create `name`, optionally as a copy of `template`.
    ///
    /// Copying a migrated template is how tests get a fresh schema without
    /// running migrations again. The template must have no open connections.
    pub async fn create_database(&self, name: &str, template: Option<&str>) -> Result<()> {
        let mut sql = format!("CREATE DATABASE {}", identifier(name)?);
        if let Some(template) = template {
            // FILE_COPY copies the files directly instead of writing every
            // block of the template into WAL; its checkpoint is cheap with
            // fsync off.
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

    /// Drop `name`, disconnecting anything still connected to it.
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
        let root = runtime_root()?;
        sweep_stale(&root);
        let bin = install().await?;
        let run_as = is_root().then_some(NOBODY);

        let dir = root.join(std::process::id().to_string());
        if dir.exists() {
            fs::remove_dir_all(&dir).ok();
        }
        fs::create_dir_all(&dir)?;
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))?;
        let password = uuid::Uuid::new_v4().simple().to_string();
        let pwfile = dir.join("pwfile");
        fs::write(&pwfile, &password)?;
        if let Some(uid) = run_as {
            std::os::unix::fs::chown(&dir, Some(uid), Some(uid))?;
            std::os::unix::fs::chown(&pwfile, Some(uid), Some(uid))?;
        }

        let data = dir.join("data");
        let initdb = tokio::task::spawn_blocking({
            let mut cmd = command(&bin.join("initdb"), run_as);
            cmd.arg("-D")
                .arg(&data)
                .args(["-U", SUPERUSER, "-A", "scram-sha-256", "-E", "UTF8"])
                .args(["--locale=C", "--no-sync", "--no-instructions"])
                .arg(format!("--pwfile={}", pwfile.display()));
            move || cmd.output()
        })
        .await??;
        fs::remove_file(&pwfile).ok();
        if !initdb.status.success() {
            bail!(
                "initdb failed: {}",
                String::from_utf8_lossy(&initdb.stderr).trim()
            );
        }

        let port = free_port()?;
        let log = dir.join("postgres.log");
        let mut cmd = command(&bin.join("postgres"), run_as);
        cmd.arg("-D")
            .arg(&data)
            .arg("-p")
            .arg(port.to_string())
            .arg("-c")
            .arg("listen_addresses=127.0.0.1")
            .arg("-c")
            .arg(format!("unix_socket_directories={}", dir.display()))
            // Throwaway data: trade durability for speed.
            .args(["-c", "fsync=off", "-c", "synchronous_commit=off"])
            .args(["-c", "full_page_writes=off", "-c", "max_connections=500"])
            // Nothing replicates or archives, so log only what crash recovery
            // needs and recycle WAL early; a test run creates hundreds of
            // databases and would otherwise keep gigabytes of WAL.
            .args(["-c", "wal_level=minimal", "-c", "max_wal_senders=0"])
            .args(["-c", "max_wal_size=128MB", "-c", "min_wal_size=32MB"])
            .stdout(Stdio::from(fs::File::create(&log)?))
            .stderr(Stdio::from(fs::File::options().append(true).open(&log)?));
        spawn_tied_to_process(cmd)?;

        let pg = Self {
            port,
            password,
            dir,
        };
        pg.wait_ready().await?;
        tracing::info!(port, dir = %pg.dir.display(), "embedded PostgreSQL started");
        Ok(pg)
    }

    async fn wait_ready(&self) -> Result<()> {
        let started = Instant::now();
        loop {
            match sqlx::PgConnection::connect(&self.url("postgres")).await {
                Ok(conn) => {
                    conn.close().await.ok();
                    return Ok(());
                }
                Err(err) if started.elapsed() > READY_TIMEOUT => {
                    let log = fs::read_to_string(self.dir.join("postgres.log")).unwrap_or_default();
                    let tail: Vec<_> = log.lines().rev().take(20).collect();
                    let tail: Vec<_> = tail.into_iter().rev().collect();
                    bail!(
                        "embedded PostgreSQL did not accept connections: {err}\n{}",
                        tail.join("\n")
                    );
                }
                Err(_) => tokio::time::sleep(Duration::from_millis(50)).await,
            }
        }
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

fn is_root() -> bool {
    // SAFETY: geteuid has no preconditions and cannot fail.
    unsafe { libc::geteuid() == 0 }
}

fn command(program: &Path, run_as: Option<u32>) -> Command {
    let mut cmd = Command::new(program);
    cmd.stdin(Stdio::null()).env("LC_ALL", "C");
    if let Some(uid) = run_as {
        cmd.uid(uid).gid(uid);
    }
    cmd
}

/// Spawn `cmd` so it cannot outlive this process.
///
/// The child is spawned from a thread that then waits on it, because
/// `PR_SET_PDEATHSIG` fires when the spawning *thread* exits, and the first
/// caller is usually a short-lived test or runtime thread.
fn spawn_tied_to_process(mut cmd: Command) -> Result<()> {
    #[cfg(target_os = "linux")]
    // SAFETY: the closure only calls the async-signal-safe `prctl`. std runs
    // it after switching uid, which would otherwise clear the setting.
    unsafe {
        cmd.pre_exec(|| {
            if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("embedded-postgres".to_string())
        .spawn(move || match cmd.spawn() {
            Ok(mut child) => {
                tx.send(Ok(())).ok();
                child.wait().ok();
            }
            Err(err) => {
                tx.send(Err(err)).ok();
            }
        })?;
    rx.recv()?.context("starting postgres")
}

fn free_port() -> Result<u16> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    Ok(listener.local_addr()?.port())
}

/// Parent of every process's cluster directory. World-writable and sticky,
/// like `/tmp`, so root and non-root processes can share it.
fn runtime_root() -> Result<PathBuf> {
    let root = std::env::temp_dir().join("everruns-pg");
    if !root.exists() {
        fs::create_dir_all(&root)?;
        fs::set_permissions(&root, fs::Permissions::from_mode(0o1777)).ok();
    }
    Ok(root)
}

/// Remove clusters left by processes that are gone, stopping any server a
/// platform without `PR_SET_PDEATHSIG` left running.
fn sweep_stale(root: &Path) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let Some(owner) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<i32>().ok())
        else {
            continue;
        };
        if owner as u32 == std::process::id() || process_alive(owner) {
            continue;
        }
        let path = entry.path();
        if let Some(server) = fs::read_to_string(path.join("data/postmaster.pid"))
            .ok()
            .and_then(|pid| pid.lines().next()?.trim().parse::<i32>().ok())
            && process_alive(server)
        {
            // SAFETY: plain signal delivery to a pid read from the cluster.
            unsafe { libc::kill(server, libc::SIGQUIT) };
        }
        fs::remove_dir_all(&path).ok();
    }
}

fn process_alive(pid: i32) -> bool {
    // SAFETY: signal 0 only checks that the pid exists.
    let alive = unsafe { libc::kill(pid, 0) == 0 };
    alive || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

/// Directory holding the installed binaries, downloading them on first use.
async fn install() -> Result<PathBuf> {
    let cache = cache_dir()?;
    if let Some(installed) = installed_version(&cache) {
        return Ok(cache.join(installed.to_string()).join("bin"));
    }
    let url = postgresql_archive::configuration::zonky::URL;
    let req = VersionReq::parse(&format!("^{MAJOR_VERSION}"))?;
    tracing::info!(version = %req, "downloading PostgreSQL binaries");
    // Maven Central rate-limits bursts (429); a short backoff gets through.
    let mut attempt = 0;
    let (version, bytes) = loop {
        match postgresql_archive::get_archive(url, &req).await {
            Ok(archive) => break archive,
            Err(err) if attempt < 4 => {
                attempt += 1;
                tracing::warn!(%err, attempt, "PostgreSQL download failed, retrying");
                tokio::time::sleep(Duration::from_secs(1 << attempt)).await;
            }
            Err(err) => return Err(err).context("downloading PostgreSQL binaries"),
        }
    };
    let out = cache.join(version.to_string());
    postgresql_archive::extract(url, &bytes, &out)
        .await
        .context("extracting PostgreSQL binaries")?;
    // The extractor stages through a private temp dir; open it up so a
    // cluster running as another user can execute the binaries.
    fs::set_permissions(&out, fs::Permissions::from_mode(0o755))?;
    Ok(out.join("bin"))
}

fn installed_version(cache: &Path) -> Option<Version> {
    fs::read_dir(cache)
        .ok()?
        .flatten()
        .filter_map(|entry| Version::parse(entry.file_name().to_str()?).ok())
        .filter(|version| version.major == MAJOR_VERSION)
        .filter(|version| {
            cache
                .join(version.to_string())
                .join("bin/postgres")
                .exists()
        })
        .max()
}

fn cache_dir() -> Result<PathBuf> {
    if let Some(dir) = std::env::var_os("EVERRUNS_PG_CACHE") {
        return Ok(PathBuf::from(dir));
    }
    let dir = if is_root() {
        // Root's home is private; the cluster user could not read it.
        std::env::temp_dir().join("everruns-pg-cache")
    } else if let Some(xdg) = std::env::var_os("XDG_CACHE_HOME") {
        PathBuf::from(xdg).join("everruns/postgres")
    } else {
        let home = std::env::var_os("HOME").context("HOME is not set")?;
        PathBuf::from(home).join(".cache/everruns/postgres")
    };
    fs::create_dir_all(&dir)?;
    if is_root() {
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).ok();
    }
    Ok(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifiers_are_quoted_and_validated() {
        assert_eq!(
            identifier("everruns_test_1").unwrap(),
            "\"everruns_test_1\""
        );
        assert!(identifier("").is_err());
        assert!(identifier("x\"; DROP DATABASE postgres; --").is_err());
        assert!(identifier("Upper").is_err());
    }

    #[test]
    fn installed_version_picks_the_newest_complete_install() {
        let cache = tempfile::tempdir().unwrap();
        for (version, complete) in [("17.4.0", true), ("17.6.0", true), ("17.9.0", false)] {
            let bin = cache.path().join(version).join("bin");
            fs::create_dir_all(&bin).unwrap();
            if complete {
                fs::write(bin.join("postgres"), "").unwrap();
            }
        }
        fs::create_dir_all(cache.path().join("16.9.0/bin")).unwrap();
        fs::write(cache.path().join("16.9.0/bin/postgres"), "").unwrap();
        assert_eq!(
            installed_version(cache.path()),
            Some(Version::new(17, 6, 0))
        );
    }

    #[test]
    fn sweep_removes_dirs_of_dead_processes_only() {
        let root = tempfile::tempdir().unwrap();
        let mine = root.path().join(std::process::id().to_string());
        // pid_max is far below i32::MAX, so this owner cannot be alive.
        let dead = root.path().join((i32::MAX - 7).to_string());
        let other = root.path().join("not-a-pid");
        for dir in [&mine, &dead, &other] {
            fs::create_dir_all(dir).unwrap();
        }
        sweep_stale(root.path());
        assert!(mine.exists());
        assert!(!dead.exists());
        assert!(other.exists());
    }

    #[tokio::test]
    async fn shared_cluster_serves_queries_and_template_copies() {
        let pg = EmbeddedPostgres::shared()
            .await
            .expect("start embedded postgres");
        assert!(std::ptr::eq(pg, EmbeddedPostgres::shared().await.unwrap()));

        pg.create_database("tmpl", None).await.unwrap();
        let mut conn = sqlx::PgConnection::connect(&pg.url("tmpl")).await.unwrap();
        sqlx::raw_sql("CREATE TABLE t (id int PRIMARY KEY); INSERT INTO t VALUES (1)")
            .execute(&mut conn)
            .await
            .unwrap();
        conn.close().await.unwrap();

        pg.create_database("copy", Some("tmpl")).await.unwrap();
        let mut conn = sqlx::PgConnection::connect(&pg.url("copy")).await.unwrap();
        let (count,): (i64,) = sqlx::query_as("SELECT count(*) FROM t")
            .fetch_one(&mut conn)
            .await
            .unwrap();
        assert_eq!(count, 1);

        // Dropping disconnects the open session instead of failing on it.
        pg.drop_database("copy").await.unwrap();
        assert!(conn.ping().await.is_err());
        assert!(sqlx::PgConnection::connect(&pg.url("copy")).await.is_err());
    }
}
