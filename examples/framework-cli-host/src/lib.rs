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
//! everruns fleet get --name api
//! everruns fleet scale --name api --replicas 4
//! everruns fleet --help
//! ```

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use everruns_integrations_bashkit::cli::{
    CliCommandSource, CliCommandSourceHandle, CliCommandSpec, CliRoute,
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

const LIST: CliRoute = CliRoute::new(&["fleet"], "list").with_examples(&["everruns fleet list"]);
const GET: CliRoute =
    CliRoute::new(&["fleet"], "get").with_examples(&["everruns fleet get --name api"]);
const SCALE: CliRoute = CliRoute::new(&["fleet"], "scale")
    .with_examples(&["everruns fleet scale --name api --replicas 4"]);

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

    fn required<'a>(params: &'a Value, field: &str) -> Result<&'a str, String> {
        params
            .get(field)
            .and_then(Value::as_str)
            .ok_or_else(|| format!("missing --{field}"))
    }
}

#[async_trait]
impl CliCommandSource for FleetCommands {
    fn specs(&self) -> Vec<CliCommandSpec> {
        vec![
            CliCommandSpec {
                wire_name: "list_services".into(),
                description: "List services and their replica counts.".into(),
                route: LIST,
            },
            CliCommandSpec {
                wire_name: "get_service".into(),
                description: "Show one service.".into(),
                route: GET,
            },
            CliCommandSpec {
                wire_name: "scale_service".into(),
                description: "Set a service's replica count.".into(),
                route: SCALE,
            },
        ]
    }

    fn node_about(&self) -> Vec<(String, String)> {
        vec![(
            "fleet".to_string(),
            "Services this deployment runs, and their scale.".to_string(),
        )]
    }

    fn usage(&self, wire_name: &str, display_name: &str) -> String {
        // A source renders its own flags. A server generates this from each
        // command's JSON Schema; a small application can spell them out.
        let flags = match wire_name {
            "get_service" => " --name <service>",
            "scale_service" => " --name <service> --replicas <count>",
            _ => "",
        };
        format!("Usage: {display_name}{flags}\n")
    }

    async fn dispatch(&self, wire_name: &str, params: Value) -> Result<String, String> {
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
                let name = Self::required(&params, "name")?;
                let services = self
                    .fleet
                    .services
                    .lock()
                    .map_err(|_| "fleet unavailable")?;
                match services.get(name) {
                    Some(replicas) => Ok(json!({ "name": name, "replicas": replicas }).to_string()),
                    // Errors name the real options so a caller can correct
                    // itself without asking a human.
                    None => Err(format!(
                        "unknown service `{name}`. Known: {}",
                        services.keys().cloned().collect::<Vec<_>>().join(", ")
                    )),
                }
            }
            "scale_service" => {
                let name = Self::required(&params, "name")?.to_string();
                let replicas = params
                    .get("replicas")
                    .and_then(|value| match value {
                        Value::String(text) => text.parse::<u32>().ok(),
                        Value::Number(number) => number.as_u64().map(|n| n as u32),
                        _ => None,
                    })
                    .ok_or("missing or non-numeric --replicas")?;

                let mut services = self
                    .fleet
                    .services
                    .lock()
                    .map_err(|_| "fleet unavailable")?;
                if !services.contains_key(&name) {
                    return Err(format!("unknown service `{name}`"));
                }
                services.insert(name.clone(), replicas);
                Ok(json!({ "name": name, "replicas": replicas, "scaled": true }).to_string())
            }
            other => Err(format!("unknown command `{other}`")),
        }
    }
}
