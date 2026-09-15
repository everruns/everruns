use std::path::Path;
use tokio::{
    process::Command,
    time::{Duration, timeout},
};

fn source(path: &str) -> Result<&'static str, String> {
    match path {
        "sample_payment.rs" => Ok(include_str!("sample_payment.rs")),
        "contract.md" => Ok(include_str!("contract.md")),
        "regression.rs" => Ok(include_str!("regression.rs")),
        _ => Err("Only sample_payment.rs, contract.md, and regression.rs are exposed.".into()),
    }
}

#[everruns::tool]
/// Read a bundled source file, refund contract, or regression test.
pub async fn inspect_change(path: String) -> Result<String, String> {
    source(&path).map(str::to_owned)
}

async fn reproduce() -> Result<String, String> {
    let directory = tempfile::tempdir().map_err(|e| e.to_string())?;
    let binary = directory.path().join("refund-regression");
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    // Fixed trusted fixture only: no model-supplied commands, source, or paths.
    let compile = timeout(
        Duration::from_secs(30),
        Command::new("rustc")
            .args(["--test", "--edition=2024", "-Awarnings"])
            .arg("regression.rs")
            .arg("-o")
            .arg(&binary)
            .current_dir(fixtures)
            .kill_on_drop(true)
            .output(),
    )
    .await
    .map_err(|_| "Compilation timed out")?
    .map_err(|e| e.to_string())?;
    if !compile.status.success() {
        return Err(format!(
            "Compiler failed: {}",
            String::from_utf8_lossy(&compile.stderr)
        ));
    }
    let output = timeout(
        Duration::from_secs(10),
        Command::new(&binary)
            .arg("--nocapture")
            .kill_on_drop(true)
            .output(),
    )
    .await
    .map_err(|_| "Regression timed out")?
    .map_err(|e| e.to_string())?;
    Ok(format!(
        "Regression exit code: {}\n{}\n{}",
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    ))
}

#[everruns::tool]
/// Compile and execute the bundled cumulative-refund regression, without modifying code.
pub async fn run_regression() -> Result<String, String> {
    reproduce().await
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn reproduces_the_actual_refund_defect() {
        let report = reproduce().await.unwrap();
        assert!(report.contains("Regression exit code: 101"));
        assert!(report.contains("left: 2000"));
        assert!(report.contains("right: 1000"));
    }
    #[test]
    fn refuses_arbitrary_files() {
        assert!(source("../../Cargo.toml").is_err());
        assert!(source("/etc/passwd").is_err());
        assert!(source("contract.md").unwrap().contains("cumulative"));
    }
}
