use super::*;

#[test]
fn worker_text_breaks_on_newlines_and_then_on_words() {
    let mut buffer = String::from("first\nsecond");
    assert_eq!(next_line(&mut buffer).as_deref(), Some("first"));
    assert_eq!(next_line(&mut buffer), None);
    assert_eq!(buffer, "second");

    // A single long chunk still breaks, and breaks between words.
    let mut buffer = "lorem ipsum ".repeat(30);
    let line = next_line(&mut buffer).unwrap();
    assert!(line.chars().count() <= WRAP, "{line}");
    assert!(line.ends_with("ipsum") || line.ends_with("lorem"), "{line}");
    assert!(!buffer.starts_with(' '));
}

#[test]
fn a_word_longer_than_the_line_is_cut_rather_than_held_forever() {
    let mut buffer = "x".repeat(WRAP + 10);
    let line = next_line(&mut buffer).unwrap();
    assert_eq!(line.chars().count(), WRAP);
    assert_eq!(buffer.chars().count(), 10);
}

#[test]
fn every_dimension_has_a_label_that_fits_its_column() {
    for dimension in &DIMENSIONS {
        assert!(dimension.id.len() <= LABEL, "{}", dimension.id);
    }
}
