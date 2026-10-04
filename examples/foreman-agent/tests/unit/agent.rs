use super::*;

#[test]
fn both_agents_build_without_contacting_a_provider() {
    let workspace = tempfile::tempdir().unwrap();
    let model = Model::simulated("ready");
    assert!(worker(model.clone(), workspace.path()).is_ok());
    assert!(verifier(model, workspace.path()).is_ok());
}

#[test]
fn missions_carry_the_job_and_differ_in_posture() {
    let job = "Add tiered shipping rates.";
    assert!(coding_mission(job).contains(job));
    assert!(verification_mission(job).contains(job));
    assert!(verification_mission(job).contains("do not attempt repairs"));
}
