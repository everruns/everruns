use super::*;

#[test]
fn scope_shape_is_not_an_identity_proof() {
    for (org_id, holder, fence) in [(0, "executor", 1), (2, " ", 1), (2, "executor", 0)] {
        assert!(
            ActorJournalScope::new(
                uuid::Uuid::new_v4(),
                uuid::Uuid::new_v4(),
                org_id,
                SessionId::new(),
                holder.into(),
                fence
            )
            .is_err()
        );
    }
}

#[test]
fn a_scope_requires_non_nil_grant_and_executor_references() {
    for (grant, executor) in [
        (uuid::Uuid::nil(), uuid::Uuid::new_v4()),
        (uuid::Uuid::new_v4(), uuid::Uuid::nil()),
    ] {
        assert!(
            ActorJournalScope::new(grant, executor, 2, SessionId::new(), "executor".into(), 1)
                .is_err()
        );
    }
}
