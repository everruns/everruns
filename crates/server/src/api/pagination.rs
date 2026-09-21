pub fn bounded_page_limit(limit: Option<u32>, default: u32, max: u32) -> Result<u32, String> {
    let limit = limit.unwrap_or(default);
    if (1..=max).contains(&limit) {
        Ok(limit)
    } else {
        Err(format!("limit must be between 1 and {max}"))
    }
}
