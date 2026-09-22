//! Model-family gating tests (adaptive thinking, `[1m]` twins), split out of
//! `driver.rs` to keep that file under its size ratchet.

use super::*;

#[test]
fn test_uses_adaptive_thinking_by_family() {
    // Adaptive-only / adaptive-recommended families, with and without
    // dated suffixes.
    assert!(uses_adaptive_thinking("claude-fable-5-1"));
    assert!(uses_adaptive_thinking("claude-fable-5-1-20260901"));
    assert!(uses_adaptive_thinking("claude-fable-5"));
    assert!(uses_adaptive_thinking("claude-fable-5-20260601"));
    assert!(uses_adaptive_thinking("claude-opus-5-5"));
    assert!(uses_adaptive_thinking("claude-opus-5"));
    assert!(uses_adaptive_thinking("claude-opus-5-20260101"));
    assert!(uses_adaptive_thinking("claude-opus-4-8"));
    assert!(uses_adaptive_thinking("claude-opus-4-7-20260416"));
    assert!(uses_adaptive_thinking("claude-opus-4-6"));
    assert!(uses_adaptive_thinking("claude-sonnet-5"));
    assert!(uses_adaptive_thinking("claude-sonnet-4-6"));
    // Budget-based families stay on extended thinking.
    assert!(!uses_adaptive_thinking("claude-opus-4-5"));
    assert!(!uses_adaptive_thinking("claude-sonnet-4-5"));
    assert!(!uses_adaptive_thinking("claude-haiku-4-5-20251001"));
}

#[test]
fn test_split_million_context() {
    // Registered `[1m]` twins: stripped to the wire id and flagged.
    assert_eq!(
        split_million_context("claude-opus-4-8[1m]"),
        ("claude-opus-4-8", true)
    );
    assert_eq!(
        split_million_context("claude-fable-5-1[1m]"),
        ("claude-fable-5-1", true)
    );
    assert_eq!(
        split_million_context("claude-fable-5[1m]"),
        ("claude-fable-5", true)
    );
    assert_eq!(
        split_million_context("claude-opus-5[1m]"),
        ("claude-opus-5", true)
    );
    assert_eq!(
        split_million_context("claude-opus-5-5[1m]"),
        ("claude-opus-5-5", true)
    );
    assert_eq!(
        split_million_context("claude-opus-4-6[1m]"),
        ("claude-opus-4-6", true)
    );
    assert_eq!(
        split_million_context("claude-sonnet-5[1m]"),
        ("claude-sonnet-5", true)
    );

    // Date-suffixed 1M-capable id is still honored (family normalization).
    assert_eq!(
        split_million_context("claude-opus-4-8-20260101[1m]"),
        ("claude-opus-4-8-20260101", true)
    );

    // Bare ids are unchanged and not flagged.
    assert_eq!(
        split_million_context("claude-opus-4-8"),
        ("claude-opus-4-8", false)
    );

    // Models that merely end in `[1m]` but are NOT 1M-capable must be left
    // untouched — never strip them or send `context-1m` (it can 400 or
    // silently truncate, e.g. on sonnet-4-5 where the header was retired).
    for not_1m in [
        "claude-haiku-4-5[1m]",
        "claude-haiku-4-5-20251001[1m]",
        "claude-sonnet-4-5[1m]",
        "totally-made-up[1m]",
    ] {
        assert_eq!(split_million_context(not_1m), (not_1m, false), "{not_1m}");
    }
}
