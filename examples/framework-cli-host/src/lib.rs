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
        // A command declares its arguments as a JSON Schema and gets a parser
        // and a `--help` for free: required fields are enforced, `--replicas`
        // arrives as a number rather than as text this source has to coerce,
        // and a misspelled flag is rejected with the usage block attached. A
        // server generates these schemas from its command types; an
        // application this size writes them out.
        vec![
            CliCommandSpec {
                wire_name: "list_services".into(),
                description: "List services and their replica counts.".into(),
                route: LIST,
                params: json!({ "type": "object", "properties": {} }),
                positional: None,
            },
            CliCommandSpec {
                wire_name: "get_service".into(),
                description: "Show one service.".into(),
                route: GET,
                params: json!({
                    "type": "object",
                    "properties": {
                        "name": { "type": "string", "description": "Service name." }
                    },
                    "required": ["name"]
                }),
                // `everruns fleet get api` reads better than `--name api`, and
                // costs one declaration.
                positional: Some("name".into()),
            },
            CliCommandSpec {
                wire_name: "scale_service".into(),
                description: "Set a service's replica count.".into(),
                route: SCALE,
                params: json!({
                    "type": "object",
                    "properties": {
                        "name": { "type": "string", "description": "Service name." },
                        "replicas": {
                            "type": "integer",
                            "description": "Desired replica count."
                        }
                    },
                    "required": ["name", "replicas"]
                }),
                positional: Some("name".into()),
            },
        ]
    }

    fn node_about(&self) -> Vec<(String, String)> {
        vec![(
            "fleet".to_string(),
            "Services this deployment runs, and their scale.".to_string(),
        )]
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
                // Typed by the schema before dispatch: a non-numeric
                // `--replicas` never reaches here, and neither does a missing
                // one, so there is no coercion to write.
                let replicas = params
                    .get("replicas")
                    .and_then(Value::as_u64)
                    .ok_or("missing or non-numeric --replicas")?
                    as u32;

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
