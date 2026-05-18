// Unified-diff helper shared by the approval prompt and the tool-result
// rendering. Extracted to a tiny module so both call sites stay readable.

use similar::TextDiff;

pub fn unified(old: &str, new: &str, label: &str, context: usize) -> String {
    let diff = TextDiff::from_lines(old, new);
    diff.unified_diff()
        .header(&format!("a/{label}"), &format!("b/{label}"))
        .context_radius(context)
        .to_string()
}
