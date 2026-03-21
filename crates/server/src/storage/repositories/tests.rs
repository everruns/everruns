use super::*;

#[test]
fn database_pool_config_defaults() {
    let config = DatabasePoolConfig::default();
    assert_eq!(config.max_connections, 50);
    assert_eq!(config.min_connections, 5);
    assert_eq!(config.acquire_timeout, std::time::Duration::from_secs(5));
    assert_eq!(config.idle_timeout, std::time::Duration::from_secs(300));
}

#[test]
fn database_pool_config_from_env() {
    // SAFETY: test is run single-threaded (--test-threads=1)
    unsafe {
        std::env::set_var("DATABASE_POOL_MAX", "100");
        std::env::set_var("DATABASE_POOL_MIN", "10");
        std::env::set_var("DATABASE_ACQUIRE_TIMEOUT_SECS", "15");
        std::env::set_var("DATABASE_IDLE_TIMEOUT_SECS", "600");
    }

    let config = DatabasePoolConfig::from_env();
    assert_eq!(config.max_connections, 100);
    assert_eq!(config.min_connections, 10);
    assert_eq!(config.acquire_timeout, std::time::Duration::from_secs(15));
    assert_eq!(config.idle_timeout, std::time::Duration::from_secs(600));

    unsafe {
        std::env::remove_var("DATABASE_POOL_MAX");
        std::env::remove_var("DATABASE_POOL_MIN");
        std::env::remove_var("DATABASE_ACQUIRE_TIMEOUT_SECS");
        std::env::remove_var("DATABASE_IDLE_TIMEOUT_SECS");
    }
}

#[test]
fn database_pool_config_invalid_values_use_defaults() {
    // SAFETY: test is run single-threaded (--test-threads=1)
    unsafe {
        std::env::set_var("DATABASE_POOL_MAX", "not_a_number");
    }

    let config = DatabasePoolConfig::from_env();
    assert_eq!(config.max_connections, 50); // falls back to default

    unsafe {
        std::env::remove_var("DATABASE_POOL_MAX");
    }
}

// ─── build_search_sql / escape_like tests ───

#[test]
fn escape_like_leaves_normal_text_unchanged() {
    assert_eq!(escape_like("hello world"), "hello world");
}

#[test]
fn escape_like_escapes_percent() {
    assert_eq!(escape_like("100%"), "100\\%");
}

#[test]
fn escape_like_escapes_underscore() {
    assert_eq!(escape_like("a_b"), "a\\_b");
}

#[test]
fn escape_like_escapes_backslash() {
    assert_eq!(escape_like("c:\\path"), "c:\\\\path");
}

#[test]
fn escape_like_handles_all_special_chars_together() {
    assert_eq!(escape_like("%_\\"), "\\%\\_\\\\");
}

#[test]
fn build_search_sql_none_returns_empty() {
    let (sql, patterns) = build_search_sql(None, "LOWER(name)", 2);
    assert!(sql.is_empty());
    assert!(patterns.is_empty());
}

#[test]
fn build_search_sql_empty_string_returns_empty() {
    let (sql, patterns) = build_search_sql(Some(""), "LOWER(name)", 2);
    assert!(sql.is_empty());
    assert!(patterns.is_empty());
}

#[test]
fn build_search_sql_whitespace_only_returns_empty() {
    let (sql, patterns) = build_search_sql(Some("   "), "LOWER(name)", 2);
    assert!(sql.is_empty());
    assert!(patterns.is_empty());
}

#[test]
fn build_search_sql_single_word() {
    let (sql, patterns) = build_search_sql(Some("hello"), "LOWER(name)", 2);
    assert_eq!(patterns, vec!["%hello%"]);
    assert!(sql.contains("LIKE $2 ESCAPE"));
    assert!(!sql.contains("$3"));
}

#[test]
fn build_search_sql_multi_word() {
    let (sql, patterns) = build_search_sql(Some("hello world"), "LOWER(name)", 2);
    assert_eq!(patterns, vec!["%hello%", "%world%"]);
    assert!(sql.contains("LIKE $2 ESCAPE"));
    assert!(sql.contains("LIKE $3 ESCAPE"));
}

#[test]
fn build_search_sql_escapes_like_wildcards() {
    let (_, patterns) = build_search_sql(Some("100% done"), "LOWER(name)", 2);
    assert_eq!(patterns, vec!["%100\\%%", "%done%"]);
}

#[test]
fn build_search_sql_escapes_underscore() {
    let (_, patterns) = build_search_sql(Some("my_var"), "LOWER(name)", 2);
    assert_eq!(patterns, vec!["%my\\_var%"]);
}

#[test]
fn build_search_sql_caps_tokens_at_max() {
    // A poem or long query should be capped
    let poem = "roses are red violets are blue sugar is sweet and so are you extra words here";
    let (_, patterns) = build_search_sql(Some(poem), "LOWER(name)", 2);
    assert_eq!(patterns.len(), MAX_SEARCH_TOKENS);
}

#[test]
fn build_search_sql_unicode_tokens() {
    let (_, patterns) = build_search_sql(Some("日本語 テスト"), "LOWER(name)", 2);
    assert_eq!(patterns, vec!["%日本語%", "%テスト%"]);
}

#[test]
fn build_search_sql_emoji_input() {
    let (_, patterns) = build_search_sql(Some("🤖 bot"), "LOWER(name)", 2);
    assert_eq!(patterns, vec!["%🤖%", "%bot%"]);
}

#[test]
fn build_search_sql_sql_injection_attempt() {
    // SQL injection via search should be harmless — values are parameterized
    let (sql, patterns) = build_search_sql(Some("'; DROP TABLE agents; --"), "LOWER(name)", 2);
    // The tokens are just words, bound as parameters
    assert_eq!(patterns.len(), 5); // "';", "drop", "table", "agents;", "--"
    // SQL structure is safe — only LIKE clauses
    assert!(sql.contains("LIKE $2"));
    assert!(!sql.contains("DROP"));
}

#[test]
fn build_search_sql_param_offset() {
    // When starting at param 4 (e.g. after org_id, agent_id, other filters)
    let (sql, _) = build_search_sql(Some("test query"), "LOWER(name)", 4);
    assert!(sql.contains("$4"));
    assert!(sql.contains("$5"));
}

#[test]
fn build_search_sql_case_insensitive() {
    let (_, patterns) = build_search_sql(Some("HeLLo WoRLd"), "LOWER(name)", 2);
    assert_eq!(patterns, vec!["%hello%", "%world%"]);
}

#[test]
fn build_search_sql_extra_whitespace_collapsed() {
    let (_, patterns) = build_search_sql(Some("  hello    world  "), "LOWER(name)", 2);
    assert_eq!(patterns, vec!["%hello%", "%world%"]);
}
