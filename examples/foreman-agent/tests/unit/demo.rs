use super::read_job;

#[test]
fn the_entered_job_is_trimmed_and_prompted() -> std::io::Result<()> {
    let mut output = Vec::new();
    let job = read_job(&mut &b"  Add tier boundary tests.  \n"[..], &mut output)?;
    assert_eq!(job, "Add tier boundary tests.");
    assert_eq!(output, b"\nJob: ");
    Ok(())
}

#[test]
fn empty_input_cannot_start_a_worker() {
    for input in ["", " \n"] {
        assert!(read_job(&mut input.as_bytes(), &mut Vec::new()).is_err());
    }
}
