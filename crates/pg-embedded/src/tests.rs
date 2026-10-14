use super::*;
use std::io::Read;
use std::net::{Ipv4Addr, SocketAddrV4, TcpStream};
use std::time::Instant;

#[test]
fn identifiers_are_quoted_and_validated() {
    assert_eq!(
        identifier("everruns_test_1").unwrap(),
        "\"everruns_test_1\""
    );
    for invalid in ["", "Upper", "x\"; DROP DATABASE postgres; --"] {
        assert!(identifier(invalid).is_err());
    }
}

// Invoke the same test executable so the fixture exercises process-global
// sharing, including process::exit and SIGKILL bypassing Rust destructors.
#[tokio::test]
#[ignore = "subprocess fixture for owner_exit_stops_postgres_and_removes_private_data"]
async fn cluster_owner_fixture() {
    let ready = PathBuf::from(std::env::var_os("EVERRUNS_PG_OWNER_READY").unwrap());
    let pg = EmbeddedPostgres::shared()
        .await
        .expect("start embedded postgres");
    assert!(std::ptr::eq(pg, EmbeddedPostgres::shared().await.unwrap()));
    let mut conn = sqlx::PgConnection::connect(&pg.url("postgres"))
        .await
        .unwrap();
    let (durability,): (String,) = sqlx::query_as("SHOW fsync")
        .fetch_one(&mut conn)
        .await
        .unwrap();
    assert_eq!(durability, "off");
    let (listeners,): (String,) = sqlx::query_as("SHOW listen_addresses")
        .fetch_one(&mut conn)
        .await
        .unwrap();
    assert_eq!(listeners, "127.0.0.1");
    conn.close().await.unwrap();

    pg.create_database("tmpl", None).await.unwrap();
    let mut conn = sqlx::PgConnection::connect(&pg.url("tmpl")).await.unwrap();
    let locale: (String, String) = sqlx::query_as(
        "SELECT datcollate, datctype FROM pg_database WHERE datname = current_database()",
    )
    .fetch_one(&mut conn)
    .await
    .unwrap();
    assert_eq!(locale, ("C".into(), "C".into()));
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
    pg.drop_database("copy").await.unwrap();
    assert!(conn.ping().await.is_err());
    assert!(sqlx::PgConnection::connect(&pg.url("copy")).await.is_err());

    let settings = pg.postgres.settings();
    let root = settings.data_dir.parent().unwrap();
    let marker = ready.with_extension("tmp");
    fs::write(&marker, format!("{}\n{}", settings.port, root.display())).unwrap();
    fs::rename(marker, ready).unwrap();
    // The parent either closes stdin or kills this process.
    std::io::stdin().read_to_end(&mut Vec::new()).unwrap();
    std::process::exit(0);
}

struct Owner(Child);

impl Drop for Owner {
    fn drop(&mut self) {
        if self.0.try_wait().unwrap().is_none() {
            self.0.kill().unwrap();
            self.0.wait().unwrap();
        }
    }
}

#[test]
fn owner_exit_stops_postgres_and_removes_private_data() {
    for termination in ["exit", "kill", "group"] {
        let work = tempfile::tempdir().unwrap();
        let ready = work.path().join("ready");
        let log = work.path().join("owner.log");
        let sentinel = work.path().join("keep");
        fs::write(&sentinel, "unrelated data").unwrap();
        let mut owner = Owner(
            Command::new(std::env::current_exe().unwrap())
                .process_group(0)
                .args([
                    "--exact",
                    "tests::cluster_owner_fixture",
                    "--ignored",
                    "--nocapture",
                ])
                .env("EVERRUNS_PG_OWNER_READY", &ready)
                .stdin(Stdio::piped())
                .stdout(Stdio::from(fs::File::create(&log).unwrap()))
                .stderr(Stdio::from(
                    fs::File::options().append(true).open(&log).unwrap(),
                ))
                .spawn()
                .unwrap(),
        );
        let deadline = Instant::now() + Duration::from_secs(60);
        while !ready.exists() {
            assert!(
                Instant::now() < deadline,
                "owner startup timed out: {}",
                fs::read_to_string(&log).unwrap()
            );
            assert!(
                owner.0.try_wait().unwrap().is_none(),
                "owner exited before readiness: {}",
                fs::read_to_string(&log).unwrap()
            );
            std::thread::sleep(Duration::from_millis(25));
        }
        let marker = fs::read_to_string(&ready).unwrap();
        let (port, root) = marker.split_once('\n').unwrap();
        let address = SocketAddrV4::new(Ipv4Addr::LOCALHOST, port.parse().unwrap());
        let root = PathBuf::from(root);
        assert!(TcpStream::connect_timeout(&address.into(), Duration::from_millis(250)).is_ok());
        match termination {
            "kill" => owner.0.kill().unwrap(),
            "group" => {
                // SAFETY: this test created the child's process group, and
                // the live child is still its leader. No unrelated process
                // can join it from another session.
                assert_eq!(
                    unsafe { libc::kill(-(owner.0.id() as i32), libc::SIGTERM) },
                    0
                );
            }
            _ => drop(owner.0.stdin.take()),
        }
        let deadline = Instant::now() + Duration::from_secs(15);
        while owner.0.try_wait().unwrap().is_none() {
            assert!(Instant::now() < deadline, "owner did not exit");
            std::thread::sleep(Duration::from_millis(25));
        }
        while root.exists()
            || TcpStream::connect_timeout(&address.into(), Duration::from_millis(100)).is_ok()
        {
            assert!(
                Instant::now() < deadline,
                "PostgreSQL survived owner termination ({termination}): {}",
                root.display()
            );
            std::thread::sleep(Duration::from_millis(25));
        }
        assert_eq!(fs::read_to_string(sentinel).unwrap(), "unrelated data");
    }
}
