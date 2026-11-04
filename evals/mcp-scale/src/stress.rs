//! Offline retrieval benchmark against the same ToolSearchIndex used by core
//! and the Tools in Shell adapter. Timings are synthetic and profile-specific.

use std::io;
use std::process::Command;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use everruns_contracts::runtime::tool_catalog::ToolSearchIndex;
use everruns_contracts::runtime::{mcp_tool_name, parse_mcp_tool_name};
use everruns_contracts::{BuiltinTool, DeferrablePolicy, ToolDefinition, ToolHints, ToolPolicy};
use serde_json::{Value, json};

use crate::{fixture, subject};

const SIZES: [usize; 4] = [50, 600, 2_000, 10_000];
const SERVER_COUNTS: [usize; 2] = [1, 10];
const BUILD_ITERATIONS: usize = 5;
const SEARCH_ITERATIONS: usize = 50;
const LIMIT: usize = 10;
const QUERIES: [(&str, &str); 3] = [
    ("ticket owner linked incident", "mcp_telemetry__get_ticket"),
    (
        "failed billing-api staging deployment rollback",
        "mcp_telemetry__list_deployments",
    ),
    (
        "geothermal drilling telemetry OPS-8421",
        "mcp_telemetry__get_ticket",
    ),
];

pub fn run(path: &str) -> io::Result<()> {
    let mut matrix = Vec::new();
    for size in SIZES {
        for server_count in SERVER_COUNTS {
            let definitions = definitions(size, server_count);
            let catalog_bytes = serde_json::to_vec(&definitions)
                .map_err(io::Error::other)?
                .len();
            let mut build_us = Vec::with_capacity(BUILD_ITERATIONS);
            let mut index = ToolSearchIndex::default();
            for _ in 0..BUILD_ITERATIONS {
                let started = Instant::now();
                index = ToolSearchIndex::new(&definitions);
                build_us.push(started.elapsed().as_secs_f64() * 1_000_000.0);
            }
            let mut query_results = Vec::new();
            for (query, expected) in QUERIES {
                let mut search_us = Vec::with_capacity(SEARCH_ITERATIONS);
                let mut observed = Vec::new();
                let mut strategy = "operations";
                for _ in 0..SEARCH_ITERATIONS {
                    let started = Instant::now();
                    let mut hits = index.search_operations(query, LIMIT, |_| true);
                    strategy = "operations";
                    if hits.is_empty() {
                        hits = index.search(query, LIMIT, |_| true);
                        strategy = "source_fallback";
                    }
                    search_us.push(started.elapsed().as_secs_f64() * 1_000_000.0);
                    observed = hits
                        .into_iter()
                        .map(|hit| json!({"name":hit.name, "score":hit.score, "tool_match":hit.tool_match, "server_match":hit.server_match}))
                        .collect::<Vec<_>>();
                }
                let operation_rank = observed
                    .iter()
                    .position(|hit| hit["name"] == expected && hit["tool_match"] == true)
                    .map(|position| position + 1);
                let mut servers = Vec::new();
                for hit in &observed {
                    if hit["server_match"] == true
                        && let Some(name) = hit["name"].as_str()
                        && let Some((server, _)) = parse_mcp_tool_name(name)
                        && !servers.contains(&server)
                    {
                        servers.push(server);
                    }
                }
                let expected_server_rank = servers
                    .iter()
                    .position(|server| server == "telemetry")
                    .map(|position| position + 1);
                let result_bytes = serde_json::to_vec(&observed)
                    .map_err(io::Error::other)?
                    .len();
                query_results.push(json!({
                    "query": query,
                    "expected_tool": expected,
                    "search_strategy": strategy,
                    "expected_operation_rank_at_10": operation_rank,
                    "expected_server_rank_at_10": expected_server_rank,
                    "operation_top1_hit": operation_rank == Some(1),
                    "server_top1_hit": expected_server_rank == Some(1),
                    "matching_servers": servers,
                    "search_iterations": SEARCH_ITERATIONS,
                    "search_us": stats(&search_us),
                    "top10_result_bytes": result_bytes,
                    "top10": observed,
                }));
            }
            matrix.push(json!({
                "catalog_tools": definitions.len(),
                "servers": server_count,
                "server_tool_counts": (0..server_count).map(|i| subject::server_catalog_size(size, server_count, i)).collect::<Vec<_>>(),
                "catalog_serialized_bytes": catalog_bytes,
                "build_iterations": BUILD_ITERATIONS,
                "index_build_us": stats(&build_us),
                "queries": query_results,
            }));
        }
    }
    let rustc = Command::new("rustc")
        .arg("--version")
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .unwrap_or_else(|| "unavailable".into());
    let report: Value = json!({
        "schema_version": 1,
        "title": "Synthetic MCP catalog stress using the production ToolSearchIndex",
        "method": "ToolSearchIndex::new and production bounded-search strategy: search_operations first, then search fallback only when there are no operation matches; populated with MCP-shaped BuiltinTool definitions",
        "synthetic_only": true,
        "timestamp_unix_ms": SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis(),
        "environment": {
            "rustc": rustc,
            "os": std::env::consts::OS,
            "arch": std::env::consts::ARCH,
            "logical_cpus": std::thread::available_parallelism().map(usize::from).unwrap_or(1),
            "build_profile": if cfg!(debug_assertions) { "dev" } else { "release" },
            "cargo_package_version": env!("CARGO_PKG_VERSION"),
        },
        "parameters": {"catalog_sizes":SIZES, "server_counts":SERVER_COUNTS, "result_limit":LIMIT, "build_iterations":BUILD_ITERATIONS, "search_iterations_per_query":SEARCH_ITERATIONS},
        "byte_measurement": "serialized production-shaped Builtin ToolDefinition records, including MCP naming, server metadata, and capability attribution, plus serialized top-10 hits; this is not a captured host transport payload and index heap allocation is not exposed by this API",
        "matrix": matrix,
    });
    let output = serde_json::to_vec_pretty(&report).map_err(io::Error::other)?;
    if let Some(parent) = std::path::Path::new(path).parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, output)
}

fn definitions(total: usize, server_count: usize) -> Vec<ToolDefinition> {
    let mut definitions = Vec::with_capacity(total);
    for index in 0..server_count {
        let server = if index == 0 {
            "telemetry".to_string()
        } else {
            format!("catalog{index}")
        };
        let server_description =
            (index == 0).then_some("Geothermal drilling telemetry and field operations records");
        for item in fixture::server_catalog(
            subject::server_catalog_size(total, server_count, index),
            index == 0,
        ) {
            let name = item["name"].as_str().unwrap_or_default();
            let description = item["description"].as_str().unwrap_or_default();
            let schema = item
                .get("inputSchema")
                .cloned()
                .unwrap_or_else(|| json!({"type":"object"}));
            let full_name = mcp_tool_name(&server, name);
            let hints = ToolHints::default()
                .with_open_world(true)
                .with_metadata(json!({
                    "mcp_server": {
                        "name": server,
                        "description": server_description.unwrap_or_default(),
                    }
                }));
            let display_name = format!(
                "{server}: {}",
                name.split(['_', '-', '.'])
                    .filter(|part| !part.is_empty())
                    .collect::<Vec<_>>()
                    .join(" ")
            );
            let capability_id = format!("mcp:{index:08x}-0000-0000-0000-000000000000");
            definitions.push(
                ToolDefinition::Builtin(BuiltinTool {
                    name: full_name,
                    display_name: Some(display_name),
                    description: description.to_owned(),
                    parameters: schema,
                    policy: ToolPolicy::Auto,
                    category: None,
                    deferrable: DeferrablePolicy::default(),
                    hints,
                    full_parameters: None,
                })
                .with_capability_attribution(capability_id, Some(server.clone())),
            );
        }
    }
    definitions
}

fn stats(samples: &[f64]) -> Value {
    let mut sorted = samples.to_vec();
    sorted.sort_by(f64::total_cmp);
    let percentile = |p: f64| -> f64 {
        if sorted.is_empty() {
            return 0.0;
        }
        let position = (p * (sorted.len() - 1) as f64).round() as usize;
        sorted[position]
    };
    json!({"p50":percentile(0.50), "p95":percentile(0.95), "min":sorted.first().copied().unwrap_or(0.0), "max":sorted.last().copied().unwrap_or(0.0), "unit":"microseconds"})
}
