use super::*;

#[tokio::test]
async fn protected_floor_keeps_verified_tenant_journal_reads_and_refuses_unknown_versions() {
    let service = Arc::new(test_worker_service().await);
    let a = assignment(&service, 2, CLIENT_A, 0xa1).await;
    let b = assignment(&service, 3, CLIENT_B, 0xb2).await;
    let mut server = Server::start(service.clone(), true, Some((CLIENT_A, KEY_A))).await;
    server.read(a.session, a.byte).await.unwrap();
    let context = server.context();
    assert_eq!(
        service.db.worker_ingress_minimum_version().await.unwrap(),
        Some(1)
    );
    server.read(a.session, a.byte).await.unwrap();
    for (session, byte) in [(b.session, a.byte), (b.session, b.byte)] {
        assert_eq!(
            server.read(session, byte).await.unwrap_err().code(),
            tonic::Code::PermissionDenied
        );
    }
    server.channel = Server::connect(&server.address, Some((CLIENT_B, KEY_B))).await;
    server.read(b.session, b.byte).await.unwrap();
    sqlx::query("UPDATE worker_ingress_protection_floor SET minimum_version=2")
        .execute(service.db.database().pool())
        .await
        .unwrap();
    assert_eq!(
        server.read(b.session, b.byte).await.unwrap_err().code(),
        tonic::Code::Unavailable
    );
    assert_eq!(
        retained_read(&service, context, a.session)
            .await
            .unwrap_err()
            .code(),
        tonic::Code::Unavailable
    );
    server.channel = Server::connect(&server.address, Some((CLIENT_A, KEY_A))).await;
    assert_eq!(
        server.read(a.session, a.byte).await.unwrap_err().code(),
        tonic::Code::Unavailable
    );
}
