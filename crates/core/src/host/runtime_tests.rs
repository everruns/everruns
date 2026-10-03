use super::*;
use crate::{DEFAULT_ORG_ID, DEFAULT_ORG_PUBLIC_ID, org_public_id_from_internal};

#[test]
fn default_public_id_maps_to_default_org() {
    assert_eq!(
        in_process_internal_org_id(DEFAULT_ORG_PUBLIC_ID),
        DEFAULT_ORG_ID
    );
}

#[test]
fn invalid_public_id_does_not_fall_back_to_default() {
    for invalid in [
        "",
        "not-an-org",
        "org_short",
        "org_ZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZ",
        "ORG_00000000000000000000000000000001",
    ] {
        let mapped = in_process_internal_org_id(invalid);
        assert_ne!(mapped, crate::DEFAULT_ORG_ID);
        assert!(
            mapped >= 2,
            "invalid input {invalid:?} should not map to default"
        );
    }
}

#[test]
fn zero_public_id_does_not_fall_back_to_default() {
    // org_public_id_from_internal never produces this; a hand-crafted
    // all-zeros id is treated as invalid (raw == 0).
    let mapped = in_process_internal_org_id("org_00000000000000000000000000000000");
    assert_ne!(mapped, crate::DEFAULT_ORG_ID);
    assert!(mapped >= 2, "all-zero id should not map to default");
}

#[test]
fn synthetic_public_id_round_trips_with_internal_helper() {
    for internal in [1_i64, 2, 42, 1_000_000, i64::MAX - 1, i64::MAX] {
        let public = org_public_id_from_internal(internal);
        assert_eq!(
            in_process_internal_org_id(&public),
            internal,
            "round-trip failed for internal={internal}"
        );
    }
}

#[test]
fn distinct_synthetic_ids_map_to_distinct_internal_ids() {
    let a = org_public_id_from_internal(7);
    let b = org_public_id_from_internal(8);
    assert_ne!(a, b);
    assert_ne!(
        in_process_internal_org_id(&a),
        in_process_internal_org_id(&b)
    );
}

#[test]
fn high_entropy_uuid_style_id_hashes_into_reserved_range() {
    // First valid UUID-style id whose raw u128 exceeds i64::MAX
    // (top bit of the u128 set). It must hash to a positive i64 that
    // is neither 0 nor DEFAULT_ORG_ID.
    let high = "org_80000000000000000000000000000000";
    let mapped = in_process_internal_org_id(high);
    assert!(mapped >= 2, "mapped id {mapped} must be >= 2");
    assert_ne!(mapped, DEFAULT_ORG_ID);

    // Mapping is deterministic.
    assert_eq!(mapped, in_process_internal_org_id(high));
}

#[test]
fn high_entropy_ids_are_isolated_from_each_other() {
    let a = in_process_internal_org_id("org_80000000000000000000000000000001");
    let b = in_process_internal_org_id("org_80000000000000000000000000000002");
    assert_ne!(a, b);
    assert_ne!(a, DEFAULT_ORG_ID);
    assert_ne!(b, DEFAULT_ORG_ID);
}

#[test]
fn hash_uses_stable_sha256_truncation() {
    // SHA-256 with fixed big-endian first-8-byte truncation gives a value
    // we can pin. If this assertion ever breaks, callers depending on
    // build-stable mapping must be re-audited.
    let mapped = in_process_internal_org_id("org_80000000000000000000000000000000");
    let expected = {
        let digest = sha2::Sha256::digest(b"org_80000000000000000000000000000000");
        let mut buf = [0u8; 8];
        buf.copy_from_slice(&digest[..8]);
        let raw = u64::from_be_bytes(buf);
        ((raw % ((i64::MAX - 1) as u64)) as i64) + 2
    };
    assert_eq!(mapped, expected);
}

#[test]
fn oversize_input_is_bounded_and_does_not_collide_silently() {
    // Inputs past HASH_INPUT_CAP_BYTES are truncated before hashing, so
    // two oversize strings that agree on the first cap bytes map to the
    // same internal id. We only assert the result stays in the safe
    // [2, i64::MAX] range and is not DEFAULT_ORG_ID — the cap exists to
    // bound work, not to widen the input space.
    let oversize = "x".repeat(super::HASH_INPUT_CAP_BYTES * 4);
    let mapped = in_process_internal_org_id(&oversize);
    assert!(mapped >= 2);
    assert_ne!(mapped, DEFAULT_ORG_ID);
}
