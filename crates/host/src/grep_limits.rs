use everruns_provider::error::{AgentLoopError, Result};
use regex::{Regex, RegexBuilder};

pub(crate) const MAX_PATTERN_LEN: usize = 1_000;
pub(crate) const MAX_REGEX_SIZE: usize = 512 * 1024;
pub(crate) const MAX_FILE_BYTES: usize = 512 * 1024;
pub(crate) const MAX_TOTAL_SCAN_BYTES: usize = 5 * 1024 * 1024;

pub(crate) fn build_regex(pattern: &str) -> Result<Regex> {
    if pattern.len() > MAX_PATTERN_LEN {
        return Err(AgentLoopError::tool(format!(
            "Regex pattern too long (max {MAX_PATTERN_LEN} characters)"
        )));
    }

    RegexBuilder::new(pattern)
        .size_limit(MAX_REGEX_SIZE)
        .build()
        .map_err(|error| AgentLoopError::tool(format!("Invalid regex pattern: {error}")))
}

pub(crate) fn validate_path_pattern(path_pattern: Option<&str>) -> Result<()> {
    if path_pattern.is_some_and(|pattern| pattern.len() > MAX_PATTERN_LEN) {
        return Err(AgentLoopError::tool(format!(
            "Path pattern too long (max {MAX_PATTERN_LEN} characters)"
        )));
    }
    Ok(())
}

pub(crate) fn account_scan(total: &mut usize, file_bytes: usize) -> Result<bool> {
    if file_bytes > MAX_FILE_BYTES {
        return Ok(false);
    }
    *total = total.saturating_add(file_bytes);
    if *total > MAX_TOTAL_SCAN_BYTES {
        return Err(AgentLoopError::tool(format!(
            "Grep scan exceeds maximum total size of {MAX_TOTAL_SCAN_BYTES} bytes"
        )));
    }
    Ok(true)
}
