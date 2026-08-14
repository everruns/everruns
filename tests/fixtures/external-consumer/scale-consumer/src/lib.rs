//! Out-of-workspace acceptance fixture for the public Scale Engine.

use everruns::{Agent, Engine, EngineCreateError, Model};
use everruns_scale::{DurableTurnDriver, PortableAgent, RegistrationPath, ScaleEngine};

/// Build a definition containing references only.
pub fn portable_definition() -> Result<PortableAgent, everruns_scale::PortableAgentError> {
    PortableAgent::builder(
        "Answer concisely.",
        "provider-model",
        RegistrationPath::new("providers/example/default")?,
        RegistrationPath::new("workspaces/example/default")?,
    )
    .tool(RegistrationPath::new("tools/example/v1")?)
    .build()
}

/// Verify that the Engine SPI rejects a local facade Agent before submission.
pub fn rejects_process_local_agent() -> Result<(), EngineCreateError> {
    let engine = ScaleEngine::from_controller(DurableTurnDriver::in_memory());
    let agent = Agent::builder()
        .instructions("local")
        .model(Model::simulated("local"))
        .build()
        .expect("static local agent is valid");
    match engine.try_create(agent) {
        Err(EngineCreateError::PortableAgentRequired {
            engine: "ScaleEngine",
            component: "everruns::Agent (process-local implementation graph)",
            registration_path: "ScaleEngine::submit(PortableAgent)",
        }) => Ok(()),
        Err(error) => Err(error),
        Ok(_) => panic!("Scale accepted a process-local Agent"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_scale_surface_compiles_and_rejects_local_code() {
        assert_eq!(portable_definition().unwrap().version, 1);
        rejects_process_local_agent().unwrap();
    }
}
