use super::*;

#[test]
fn scope_shape_is_not_an_identity_proof() {
    for (org_id, holder, fence) in [(0, "executor", 1), (2, " ", 1), (2, "executor", 0)] {
        assert!(ActorJournalScope::new(org_id, SessionId::new(), holder.into(), fence).is_err());
    }
}
