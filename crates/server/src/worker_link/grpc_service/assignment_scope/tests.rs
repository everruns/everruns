use super::*;

#[test]
fn scope_shape_is_not_an_identity_proof() {
    for (org_id, holder, fence) in [(0, "executor", 1), (2, " ", 1), (2, "executor", 0)] {
        assert!(ActorJournalScope::new(org_id, SessionId::new(), holder.into(), fence).is_err());
    }
}

#[test]
fn unknown_and_future_rpcs_do_not_inherit_scoped_access() {
    let mut request = Request::new(());
    request
        .extensions_mut()
        .insert(ActorJournalScope::new(2, SessionId::new(), "executor".into(), 1).unwrap());
    for method in ["new_future_rpc", "ReadActorJournal", "execute_command", ""] {
        assert_eq!(
            check_rpc(&request, method).unwrap_err().code(),
            tonic::Code::PermissionDenied
        );
    }
}
