use super::*;
use everruns_contracts::typed_id::SessionId;
use tonic::service::Interceptor;

fn scope() -> ActorJournalScope {
    ActorJournalScope::new(
        uuid::Uuid::new_v4(),
        uuid::Uuid::new_v4(),
        2,
        SessionId::new(),
        "executor".into(),
        1,
    )
    .unwrap()
}

#[test]
fn missing_authority_never_inherits_legacy_access() {
    let request = Request::new(());
    for rpc in ["read_actor_journal", "execute_command", "emit_event"] {
        assert_eq!(
            check_rpc(&request, rpc).unwrap_err().code(),
            tonic::Code::Unauthenticated
        );
    }
}

#[test]
fn unknown_and_future_rpcs_do_not_inherit_scoped_access() {
    let mut request = Request::new(());
    request
        .extensions_mut()
        .insert(WorkerAuthority::ActorJournal(scope()));
    check_rpc(&request, "read_actor_journal").unwrap();
    for method in ["new_future_rpc", "ReadActorJournal", "execute_command", ""] {
        assert_eq!(
            check_rpc(&request, method).unwrap_err().code(),
            tonic::Code::PermissionDenied
        );
    }
}

#[test]
fn obsolete_scope_markers_are_refused_with_or_without_canonical_authority() {
    for authority in [
        None,
        Some(WorkerAuthority::LegacyWorker),
        Some(WorkerAuthority::ActorJournal(scope())),
    ] {
        let mut extensions = Extensions::new();
        if let Some(authority) = authority {
            extensions.insert(authority);
        }
        extensions.insert(scope());
        assert_eq!(
            resolve(&extensions).unwrap_err().code(),
            tonic::Code::Unauthenticated
        );
    }
}

#[test]
fn successful_authentication_produces_explicit_legacy_or_development_authority() {
    let mut development = crate::worker_link::grpc_service::GrpcAuthInterceptor::new(None);
    let request = development.call(Request::new(())).unwrap();
    assert!(matches!(
        resolve(request.extensions()).unwrap(),
        WorkerAuthority::Development
    ));
    let mut legacy =
        crate::worker_link::grpc_service::GrpcAuthInterceptor::new(Some("fixture-token".into()));
    let mut request = Request::new(());
    request
        .metadata_mut()
        .insert("authorization", "Bearer fixture-token".parse().unwrap());
    let request = legacy.call(request).unwrap();
    assert!(matches!(
        resolve(request.extensions()).unwrap(),
        WorkerAuthority::LegacyWorker
    ));
    check_rpc(&request, "execute_command").unwrap();
}

#[test]
fn reauthentication_cannot_promote_a_scoped_context_to_legacy_or_development() {
    for expected_token in [None, Some("fixture-token".into())] {
        let mut auth = crate::worker_link::grpc_service::GrpcAuthInterceptor::new(expected_token);
        let mut request = Request::new(());
        request
            .metadata_mut()
            .insert("authorization", "Bearer fixture-token".parse().unwrap());
        request
            .extensions_mut()
            .insert(WorkerAuthority::ActorJournal(scope()));
        assert_eq!(
            auth.call(request).unwrap_err().code(),
            tonic::Code::Unauthenticated
        );
    }
}

#[test]
fn an_obsolete_adapter_cannot_be_promoted_by_reauthentication() {
    for expected_token in [None, Some("fixture-token".into())] {
        let mut auth = crate::worker_link::grpc_service::GrpcAuthInterceptor::new(expected_token);
        let mut request = Request::new(());
        request
            .metadata_mut()
            .insert("authorization", "Bearer fixture-token".parse().unwrap());
        request.extensions_mut().insert(scope());
        assert_eq!(
            auth.call(request).unwrap_err().code(),
            tonic::Code::Unauthenticated
        );
    }
}
