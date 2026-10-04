use super::*;

#[test]
fn the_headline_is_one_line_that_fits_beside_its_label() {
    assert_eq!(headline("Add\n  tiers."), "Add tiers.");
    let long = "word ".repeat(40);
    let headline = headline(&long);
    assert!(headline.chars().count() <= 88);
    assert!(!headline.contains('\n'));
    assert!(headline.ends_with('\u{2026}'));
}
