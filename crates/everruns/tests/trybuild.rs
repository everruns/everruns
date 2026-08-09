//! Compile-pass and compile-fail coverage for `#[everruns::tool]` (EVE-829).
//!
//! Pass fixtures prove supported signatures expand and compile; fail fixtures
//! pin the macro's own diagnostics for rejected signatures. The fail fixtures
//! trip the macro's `compile_error!`s (stable text), not downstream type
//! errors, so their `.stderr` is robust across compiler versions.

#[cfg(feature = "macros")]
#[test]
fn ui() {
    let t = trybuild::TestCases::new();
    t.pass("tests/ui/pass_signatures.rs");
    #[cfg(feature = "capabilities")]
    t.pass("tests/ui/pass_advanced_capability.rs");
    #[cfg(feature = "capabilities")]
    t.pass("tests/ui/pass_into_capability.rs");
    t.compile_fail("tests/ui/fail_not_async.rs");
    t.compile_fail("tests/ui/fail_receiver.rs");
    t.compile_fail("tests/ui/fail_generic.rs");
    t.compile_fail("tests/ui/fail_missing_description.rs");
    t.compile_fail("tests/ui/fail_tuple_param.rs");
    t.compile_fail("tests/ui/fail_unknown_option.rs");
    #[cfg(feature = "capabilities")]
    t.compile_fail("tests/ui/fail_capability_output_contract.rs");
}
