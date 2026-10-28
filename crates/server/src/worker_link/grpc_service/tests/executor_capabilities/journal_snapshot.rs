use super::*;

async fn wait_for_event_read(pool: &sqlx::PgPool, locker: i32) {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let waiting: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM pg_stat_activity \
                 WHERE datname=current_database() AND $1=ANY(pg_blocking_pids(pid)) \
                 AND query LIKE '%FROM events%')",
            )
            .bind(locker)
            .fetch_one(pool)
            .await
            .unwrap();
            if waiting {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("journal must reach its data read after admission");
}

#[derive(Clone, Copy, Debug)]
enum Transition {
    Capability,
    Identity,
    Grant,
    Source,
    Archive,
    Lease,
    Owner,
    Floor,
    Append,
}

async fn race(transition: Transition) {
    let service = Arc::new(test_worker_service().await);
    let a = assignment(&service, 2, CLIENT_A, 0xa1).await;
    let b = assignment(&service, 3, CLIENT_B, 0xb2).await;
    let mut server = Server::start(service.clone(), true, Some((CLIENT_A, KEY_A))).await;
    server.read(a.session, a.byte).await.unwrap();
    let context = server.context();
    let pool = service.db.database().pool();
    let mut writer = everruns_durable::begin_transaction(pool).await.unwrap();
    sqlx::query("LOCK TABLE events IN ACCESS EXCLUSIVE MODE")
        .execute(&mut *writer)
        .await
        .unwrap();
    let locker: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *writer)
        .await
        .unwrap();
    let channel = server.channel.clone();
    let session = a.session;
    let byte = a.byte;
    let pending = tokio::spawn(async move {
        let mut request = read(session, byte);
        request.get_mut().limit = 100;
        WorkerServiceClient::new(channel)
            .read_actor_journal(request)
            .await
            .map(Response::into_inner)
    });
    wait_for_event_read(pool, locker).await;
    match transition {
        Transition::Capability => {
            sqlx::query("UPDATE worker_journal_capabilities SET revoked_at=clock_timestamp() WHERE capability_sha256=$1")
                .bind(a.digest.as_slice()).execute(&mut *writer).await.unwrap();
        }
        Transition::Floor => {
            sqlx::query("UPDATE worker_ingress_protection_floor SET minimum_version=2")
                .execute(&mut *writer)
                .await
                .unwrap();
        }
        Transition::Append => {}
        other => {
            let (query, id) = match other {
                Transition::Identity => (
                    "UPDATE worker_executor_identities SET revoked_at=clock_timestamp() WHERE executor_id=$1",
                    a.executor,
                ),
                Transition::Grant => (
                    "UPDATE worker_journal_read_grants SET revoked_at=clock_timestamp() WHERE id=$1",
                    a.grant,
                ),
                Transition::Source => (
                    "UPDATE resource_grants SET revoked_at=clock_timestamp(),revision=revision+1 WHERE id=(SELECT source_grant_id FROM worker_journal_grant_dependencies WHERE grant_id=$1)",
                    a.grant,
                ),
                Transition::Archive => (
                    "UPDATE sessions SET archived_at=clock_timestamp() WHERE id=$1",
                    a.session.uuid(),
                ),
                Transition::Lease => (
                    "UPDATE session_actors SET lease_holder=NULL,lease_expires_at=NULL WHERE session_id=$1",
                    a.session.uuid(),
                ),
                Transition::Owner => ("UPDATE sessions SET org_id=3 WHERE id=$1", a.session.uuid()),
                _ => unreachable!(),
            };
            assert_eq!(
                sqlx::query(query)
                    .bind(id)
                    .execute(&mut *writer)
                    .await
                    .unwrap()
                    .rows_affected(),
                1
            );
        }
    }
    let data = serde_json::to_value(OutputMessageCompletedData::new(RuntimeMessage::assistant(
        "post-revocation-sensitive-data",
    )))
    .unwrap();
    sqlx::query(
        "INSERT INTO events (session_id,sequence,event_type,ts,context,data,metadata,tags) \
        SELECT session_id,sequence+1,event_type,clock_timestamp(),context,$2,metadata,tags \
        FROM events WHERE session_id=$1 AND sequence=1",
    )
    .bind(a.session)
    .bind(data)
    .execute(&mut *writer)
    .await
    .unwrap();
    everruns_durable::commit_transaction(writer).await.unwrap();
    let page = pending.await.unwrap().unwrap();
    assert_eq!(
        page.events.len(),
        1,
        "admission must bound the data snapshot"
    );
    assert_eq!(
        page.head_sequence,
        Some(1),
        "head metadata must use the same snapshot"
    );
    assert!(!page.has_more);
    if matches!(transition, Transition::Append) {
        let next = server.read(a.session, a.byte).await.unwrap();
        assert_eq!(next.head_sequence, Some(2));
        assert!(next.has_more);
        retained_read(&service, context, a.session).await.unwrap();
    } else {
        let code = if matches!(transition, Transition::Floor) {
            tonic::Code::Unavailable
        } else {
            tonic::Code::PermissionDenied
        };
        assert_eq!(
            retained_read(&service, context, a.session)
                .await
                .unwrap_err()
                .code(),
            code,
            "{transition:?}"
        );
        server.channel = Server::connect(&server.address, Some((CLIENT_A, KEY_A))).await;
        assert_eq!(
            server.read(a.session, a.byte).await.unwrap_err().code(),
            code,
            "{transition:?}"
        );
    }
    server.channel = Server::connect(&server.address, Some((CLIENT_B, KEY_B))).await;
    assert_eq!(
        server.read(a.session, b.byte).await.unwrap_err().code(),
        if matches!(transition, Transition::Floor) {
            tonic::Code::Unavailable
        } else {
            tonic::Code::PermissionDenied
        }
    );
    if matches!(transition, Transition::Floor) {
        assert_eq!(
            server.read(b.session, b.byte).await.unwrap_err().code(),
            tonic::Code::Unavailable
        );
    } else {
        server.read(b.session, b.byte).await.unwrap();
    }
}

#[tokio::test]
async fn journal_snapshot_capability_transition() {
    race(Transition::Capability).await;
}

#[tokio::test]
async fn journal_snapshot_identity_transition() {
    race(Transition::Identity).await;
}

#[tokio::test]
async fn journal_snapshot_grant_transition() {
    race(Transition::Grant).await;
}

#[tokio::test]
async fn journal_snapshot_source_transition() {
    race(Transition::Source).await;
}

#[tokio::test]
async fn journal_snapshot_archive_transition() {
    race(Transition::Archive).await;
}

#[tokio::test]
async fn journal_snapshot_lease_transition() {
    race(Transition::Lease).await;
}

#[tokio::test]
async fn journal_snapshot_owner_transition() {
    race(Transition::Owner).await;
}

#[tokio::test]
async fn journal_snapshot_floor_transition() {
    race(Transition::Floor).await;
}

#[tokio::test]
async fn journal_snapshot_append_transition() {
    race(Transition::Append).await;
}

#[tokio::test]
async fn journal_snapshot_scoped_read_loopback_latency() {
    let service = Arc::new(test_worker_service().await);
    let a = assignment(&service, 2, CLIENT_A, 0xa1).await;
    let server = Server::start(service, true, Some((CLIENT_A, KEY_A))).await;
    let mut elapsed = Vec::new();
    for _ in 0..100 {
        let started = std::time::Instant::now();
        let page = server.read(a.session, a.byte).await.unwrap();
        elapsed.push(started.elapsed().as_micros());
        assert_eq!(page.events.len(), 1);
        assert_eq!(page.head_sequence, Some(1));
    }
    elapsed.sort_unstable();
    println!(
        "scoped snapshot loopback: n=100 p50={}us p95={}us",
        elapsed[49], elapsed[94]
    );
}
