use everruns_foreman_agent::observation::{
    Evidence, GitEvidence, Observation, WorkerKind, WorkerRecord, build,
};
use everruns_foreman_agent::policy::Config;
use std::collections::VecDeque;
use std::time::Duration;

pub fn observation() -> Observation {
    build(Evidence {
        job: "Add tiered shipping rates.",
        run_id: "test-run",
        status: "running",
        iteration: 1,
        workers: &[WorkerRecord::new("worker-1", WorkerKind::Coding, 1, 100)],
        verification: &[],
        events: &VecDeque::new(),
        previous_assessment: None,
        previous_intervention: None,
        failures: &[],
        elapsed: Duration::from_secs(3),
        git: GitEvidence::default(),
        tests: None,
        config: &Config::default(),
    })
}

pub fn tests_pass(root: &std::path::Path) -> bool {
    std::process::Command::new("bash")
        .arg("tests/run.sh")
        .current_dir(root)
        .output()
        .is_ok_and(|output| output.status.success())
}

pub fn acceptance_pass(root: &std::path::Path) -> bool {
    std::process::Command::new("bash")
        .arg("-c")
        .arg(everruns_foreman_agent::fixture::ACCEPTANCE)
        .current_dir(root)
        .output()
        .is_ok_and(|output| output.status.success())
}
