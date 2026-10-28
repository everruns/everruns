//! Keep the smoke harness's security and HTTP-routing regressions in cargo/CI.
#[test]
fn smoke_runner_contracts() {
    let tests = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    let result = std::process::Command::new("python3")
        .args(["-m", "unittest", "discover", "-s"])
        .arg(tests)
        .output()
        .expect("Python 3 is required for the custom worker smoke harness tests");
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
}
