use serve::prelude::*;

#[derive(Serialize)]
struct Availability {
    night: String,
    free: Vec<&'static str>,
}

/// Rooms still free on a night.
#[tool]
async fn rooms(
    /// The night to check, such as `Friday` or `2026-10-16`.
    night: String,
) -> Result<Availability> {
    Ok(Availability {
        night,
        free: vec!["double", "suite"],
    })
}
