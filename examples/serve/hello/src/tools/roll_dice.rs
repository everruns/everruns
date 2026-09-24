use serve::prelude::*;

#[derive(Serialize)]
struct Roll {
    sides: u32,
    value: u32,
}

/// Roll one die with the given number of sides.
#[tool]
async fn roll_dice(
    cx: &Cx,
    /// How many sides the die has (2 to 100).
    sides: u32,
) -> Result<Roll> {
    if !(2..=100).contains(&sides) {
        bail!("a die needs between 2 and 100 sides, not {sides}");
    }
    cx.progress(format!("rolling a d{sides}"));
    // Not a CSPRNG; this is a board game.
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.subsec_nanos())
        .unwrap_or_default();
    Ok(Roll {
        sides,
        value: nanos % sides + 1,
    })
}
