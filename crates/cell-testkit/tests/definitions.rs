//! Every test agent definition builds on the test host, and a definition
//! naming what the host lacks is refused with a reason.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

use everruns_cell::Host;

#[test]
fn every_test_definition_builds() {
    let host = everruns_cell_testkit::host();
    for name in ["echo", "worker", "worker3", "slow", "sim", "lead", "files"] {
        let definition = everruns_cell_testkit::definition(name).expect(name);
        host.agent(&definition)
            .unwrap_or_else(|error| panic!("{name}: {error}"));
    }
}

#[test]
fn unknown_tools_and_providers_are_refused() {
    let host = everruns_cell_testkit::host();
    let mut definition = everruns_cell_testkit::definition("worker").unwrap();
    definition["model"]["provider"] = "openai".into();
    let Err(error) = host.agent(&definition) else {
        panic!("no openai here")
    };
    assert!(error.contains("does not serve"), "{error}");

    let mut definition = everruns_cell_testkit::definition("worker").unwrap();
    definition["tools"][0]["name"] = "launch".into();
    let Err(error) = host.agent(&definition) else {
        panic!("no such tool")
    };
    assert!(error.contains("no tool launch"), "{error}");
}

#[test]
fn the_example_agents_parse() {
    for file in ["assistant", "reviewer"] {
        let path = format!(
            "{}/../../examples/cell-assistant/agents/{file}.json",
            env!("CARGO_MANIFEST_DIR")
        );
        let definition: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        everruns_cell::host::package(&definition).unwrap_or_else(|error| panic!("{file}: {error}"));
    }
}
