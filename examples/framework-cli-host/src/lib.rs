//! A Framework host whose own operations appear as `everruns <noun> <verb>`
//! inside the agent's shell.
//!
//! The point of the exercise is that nothing here is server-specific. This
//! application has no control plane, no database, and no domain-command
//! catalog; it owns three operations over an in-memory fleet. It implements
//! [`CliCommandSource`], hands that to the runtime, and the agent gets the
//! same grammar, the same bounded help, and the same errors that the hosted
//! product's much larger tree provides.
//!
//! ```text
//! everruns fleet list
//! everruns fleet get api
//! everruns fleet scale api --replicas 4
//! everruns fleet --help
//! ```

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use clap::{CommandFactory, FromArgMatches, Parser};
use everruns_integrations_bashkit::cli::{
    CliCommandSource, CliCommandSourceHandle, CliCommandSpec,
};
use serde_json::{Value, json};

/// The application's own state. A real host would reach a database, an API, or
/// a config file here; what matters to the tree is only that dispatch is
/// async and returns text.
#[derive(Debug, Default)]
pub struct Fleet {
    services: Mutex<BTreeMap<String, u32>>,
}

impl Fleet {
    pub fn with_demo_services() -> Arc<Self> {
        let fleet = Self::default();
        {
            let mut services = fleet.services.lock().expect("fresh lock");
            services.insert("api".to_string(), 2);
            services.insert("worker".to_string(), 1);
            services.insert("scheduler".to_string(), 1);
        }
        Arc::new(fleet)
    }

    pub fn replicas(&self, name: &str) -> Option<u32> {
        self.services.lock().ok()?.get(name).copied()
    }
}

/// This application's commands, spelled with clap derive.
///
/// A host that writes each command by hand wants derive, and gets it: the tree
/// takes a `clap::Command` and hands back the `ArgMatches` clap produced, so
/// nothing here is translated through a schema or a description of a schema.
/// The hosted product builds the same values from its command catalog instead,
/// because it has hundreds of commands and their parameters are already Rust
/// types.
#[derive(Parser)]
#[command(name = "list", about = "List services and their replica counts.")]
struct ListArgs {}

#[derive(Parser)]
#[command(
    name = "get",
    about = "Show one service.",
    after_help = "Examples:\n  Check what a service is currently running at:\n    everruns fleet get api"
)]
struct GetArgs {
    /// Service name.
    name: String,
}

#[derive(Parser)]
#[command(
    name = "scale",
    about = "Set a service's replica count.",
    after_help = "Examples:\n  Take a service up to four replicas:\n    everruns fleet scale api --replicas 4"
)]
struct ScaleArgs {
    /// Service name.
    name: String,
    /// Desired replica count.
    #[arg(long, short = 'r')]
    replicas: u32,
}

/// This application's command source.
#[derive(Debug)]
pub struct FleetCommands {
    fleet: Arc<Fleet>,
}

impl FleetCommands {
    pub fn new(fleet: Arc<Fleet>) -> Self {
        Self { fleet }
    }

    /// Wrap as the handle the bashkit capability looks for on the tool
    /// context. A host that never inserts one gets no `everruns` builtin, so
    /// the surface cannot advertise a tree it has no commands for.
    pub fn handle(fleet: Arc<Fleet>) -> CliCommandSourceHandle {
        CliCommandSourceHandle(Arc::new(Self::new(fleet)))
    }
}

#[async_trait]
impl CliCommandSource for FleetCommands {
    fn specs(&self) -> Vec<CliCommandSpec> {
        fn spec<T: CommandFactory>(
            wire_name: &str,
            verb: &str,
            description: &str,
        ) -> CliCommandSpec {
            CliCommandSpec {
                wire_name: wire_name.into(),
                description: description.into(),
                path: vec!["fleet".into()],
                verb: verb.into(),
                command: T::command().color(clap::ColorChoice::Never),
            }
        }

        vec![
            spec::<ListArgs>(
                "list_services",
                "list",
                "List services and their replica counts.",
            ),
            spec::<GetArgs>("get_service", "get", "Show one service."),
            spec::<ScaleArgs>("scale_service", "scale", "Set a service's replica count."),
        ]
    }

    fn node_about(&self) -> Vec<(String, String)> {
        vec![(
            "fleet".to_string(),
            "Services this deployment runs, and their scale.".to_string(),
        )]
    }

    async fn dispatch(
        &self,
        wire_name: &str,
        mut matches: clap::ArgMatches,
    ) -> Result<String, String> {
        match wire_name {
            "list_services" => {
                let services = self
                    .fleet
                    .services
                    .lock()
                    .map_err(|_| "fleet unavailable")?;
                let rows: Vec<Value> = services
                    .iter()
                    .map(|(name, replicas)| json!({ "name": name, "replicas": replicas }))
                    .collect();
                Ok(json!({ "services": rows }).to_string())
            }
            "get_service" => {
                // Typed by the parser before dispatch, so there is no coercion
                // here and no missing-argument check: clap enforced both.
                let args =
                    GetArgs::from_arg_matches_mut(&mut matches).map_err(|e| e.to_string())?;
                let services = self
                    .fleet
                    .services
                    .lock()
                    .map_err(|_| "fleet unavailable")?;
                match services.get(&args.name) {
                    Some(replicas) => {
                        Ok(json!({ "name": args.name, "replicas": replicas }).to_string())
                    }
                    // Errors name the real options so a caller can correct
                    // itself without asking a human.
                    None => Err(format!(
                        "unknown service `{}`. Known: {}",
                        args.name,
                        services.keys().cloned().collect::<Vec<_>>().join(", ")
                    )),
                }
            }
            "scale_service" => {
                let args =
                    ScaleArgs::from_arg_matches_mut(&mut matches).map_err(|e| e.to_string())?;
                let mut services = self
                    .fleet
                    .services
                    .lock()
                    .map_err(|_| "fleet unavailable")?;
                if !services.contains_key(&args.name) {
                    return Err(format!("unknown service `{}`", args.name));
                }
                services.insert(args.name.clone(), args.replicas);
                Ok(
                    json!({ "name": args.name, "replicas": args.replicas, "scaled": true })
                        .to_string(),
                )
            }
            other => Err(format!("unknown command `{other}`")),
        }
    }
}
