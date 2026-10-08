//! Failing-first coverage for the 1 KiB auto-success budget.
//! Default auto success inline budget must be 1024 bytes so typical
//! git/gh output (1-3 KiB) stays inline in one call.

use everruns_core::tool_output_sanitizer::AUTO_SUCCESS_BUDGET;

#[test]
fn auto_success_budget_defaults_to_1kib() {
    assert_eq!(AUTO_SUCCESS_BUDGET, 1024);
}
