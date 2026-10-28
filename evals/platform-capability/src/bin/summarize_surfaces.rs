//! Recompute a surface comparison from the retained Mira JSONL, including failures.
//!
//! `cargo run --bin summarize_surfaces -- report1.jsonl report2.jsonl > summary.json`
//! accepts `-` for stdin (for example, after `gzip -dc`). No model requests occur.

use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::io::{self, BufRead};

#[derive(Default)]
struct Aggregate {
    trials: usize,
    passed: usize,
    infra: usize,
    uncached: u64,
    cached: u64,
    output: u64,
    reasoning: u64,
    cost: f64,
    cost_complete: bool,
    duration_ms: u64,
    calls: f64,
    operations: f64,
    rejected: f64,
    searches: f64,
    tool_result_bytes: f64,
    peak_tool_result_bytes: f64,
    discovery_result_bytes: f64,
    unique: f64,
    peak_input: f64,
    peak_tools: f64,
    peak_tool_bytes: f64,
    first_tools: Vec<f64>,
    first_tool_bytes: Vec<f64>,
    failures: Vec<Value>,
}

impl Aggregate {
    fn add(&mut self, row: &Value) -> Result<(), String> {
        // Mira's generic total_tokens() excludes cache reads in this adapter.
        // Summing reasoning into output would count that provider subset twice.
        if row
            .pointer("/metadata/usage_convention")
            .and_then(Value::as_str)
            != Some("everruns-disjoint-v1")
        {
            return Err(format!(
                "{}: unsupported or absent usage convention",
                row["case"]
            ));
        }
        if self.trials == 0 {
            self.cost_complete = true;
        }
        self.trials += 1;
        self.passed += usize::from(row["passed"] == true);
        self.infra += usize::from(!row["error"].is_null() && row["error_kind"] == "Infra");
        let usage = &row["usage"];
        self.uncached += usage["input_tokens"].as_u64().unwrap_or(0);
        self.cached += usage["cache_read_tokens"].as_u64().unwrap_or(0);
        self.output += usage["output_tokens"].as_u64().unwrap_or(0);
        self.reasoning += usage["reasoning_tokens"].as_u64().unwrap_or(0);
        self.cost += usage["cost_usd"].as_f64().unwrap_or(0.0);
        self.cost_complete &= row.pointer("/metadata/actual_cost_complete") == Some(&json!(true));
        self.duration_ms += row["duration_ms"].as_u64().unwrap_or(0);
        let metric = |name: &str| {
            row["metrics"][format!("platform.{name}")]
                .as_f64()
                .unwrap_or(0.0)
        };
        self.calls += metric("model_calls");
        self.operations += metric("operations");
        self.rejected += metric("rejected_operations");
        self.searches += metric("search_calls");
        self.tool_result_bytes += metric("tool_result_bytes");
        self.peak_tool_result_bytes = self
            .peak_tool_result_bytes
            .max(metric("peak_tool_result_bytes"));
        self.discovery_result_bytes += metric("discovery_result_bytes");
        self.unique += metric("unique_operations");
        self.peak_input = self.peak_input.max(metric("peak_input_tokens"));
        self.peak_tools = self.peak_tools.max(metric("peak_tool_count"));
        self.peak_tool_bytes = self.peak_tool_bytes.max(metric("peak_tool_bytes"));
        self.first_tools.push(metric("first_tool_count"));
        self.first_tool_bytes.push(metric("first_tool_bytes"));
        if row["passed"] != true {
            self.failures.push(json!({
                "case": row["case"], "trial": row["trial"], "error": row["error"],
                "scores": row["scores"].as_array().into_iter().flatten()
                    .filter(|score| score["passed"] == false && score["na"] != true)
                    .cloned().collect::<Vec<_>>(),
            }));
        }
        Ok(())
    }

    fn value(&self) -> Value {
        let mean = |total: f64| total / self.trials as f64;
        let input = self.uncached + self.cached;
        let mut first_tools = self.first_tools.clone();
        first_tools.sort_by(f64::total_cmp);
        first_tools.dedup();
        let mut first_bytes = self.first_tool_bytes.clone();
        first_bytes.sort_by(f64::total_cmp);
        first_bytes.dedup();
        json!({
            "trials": self.trials, "passed": self.passed,
            "failed": self.trials - self.passed, "infra_errors": self.infra,
            "input_tokens": self.uncached, "cache_read_tokens": self.cached,
            "total_input_tokens": input, "output_tokens": self.output,
            "reasoning_tokens_subset_of_output": self.reasoning,
            "reported_cost_usd": self.cost, "actual_cost_complete": self.cost_complete,
            "mean_total_input_tokens": mean(input as f64),
            "mean_model_calls": mean(self.calls), "model_calls": self.calls,
            "mean_duration_ms": mean(self.duration_ms as f64),
            "operations": self.operations, "rejected_operations": self.rejected,
            "search_calls": self.searches,
            "tool_result_bytes": self.tool_result_bytes,
            "peak_tool_result_bytes": self.peak_tool_result_bytes,
            "discovery_result_bytes": self.discovery_result_bytes,
            "mean_unique_operations": mean(self.unique),
            "peak_input_tokens": self.peak_input,
            "peak_tool_count": self.peak_tools, "peak_tool_bytes": self.peak_tool_bytes,
            "first_tool_counts": first_tools, "first_tool_bytes": first_bytes,
            "failures": self.failures,
        })
    }
}

#[derive(Default)]
struct Report {
    arms: BTreeMap<(String, String), Aggregate>,
    cases: BTreeMap<(String, String, String), Aggregate>,
}

impl Report {
    fn add(&mut self, row: Value) -> Result<(), String> {
        let model = row
            .pointer("/target/model")
            .and_then(Value::as_str)
            .or_else(|| row["target"].as_str())
            .ok_or("missing target model")?
            .to_string();
        let surface = row
            .pointer("/metadata/surface")
            .and_then(Value::as_str)
            .ok_or("missing surface")?
            .to_string();
        let case = row["case"].as_str().ok_or("missing case")?.to_string();
        self.arms
            .entry((model.clone(), surface.clone()))
            .or_default()
            .add(&row)?;
        self.cases
            .entry((model, surface, case))
            .or_default()
            .add(&row)
    }

    fn read(&mut self, reader: impl BufRead, source: &str) -> Result<(), String> {
        for (line, text) in reader.lines().enumerate() {
            let text = text.map_err(|error| format!("{source}: {error}"))?;
            if text.trim().is_empty() {
                continue;
            }
            let row = serde_json::from_str(&text)
                .map_err(|error| format!("{source}:{}: {error}", line + 1))?;
            self.add(row)
                .map_err(|error| format!("{source}:{}: {error}", line + 1))?;
        }
        Ok(())
    }

    fn value(&self) -> Value {
        json!({
            "usage_convention": "everruns-disjoint-v1",
            "cache_control": "observational; provider cache state is not isolated",
            "arms": self.arms.iter().map(|((target,surface), aggregate)| json!({
                "target":target, "surface":surface, "results":aggregate.value(),
            })).collect::<Vec<_>>(),
            "cases": self.cases.iter().map(|((target,surface,case), aggregate)| json!({
                "target":target, "surface":surface, "case":case, "results":aggregate.value(),
            })).collect::<Vec<_>>(),
        })
    }
}

fn main() -> Result<(), String> {
    let sources: Vec<String> = std::env::args().skip(1).collect();
    if sources.is_empty() {
        return Err("usage: summarize_surfaces report.jsonl [...] | -".into());
    }
    let mut report = Report::default();
    for source in sources {
        if source == "-" {
            report.read(io::stdin().lock(), "stdin")?;
        } else {
            let file =
                std::fs::File::open(&source).map_err(|error| format!("{source}: {error}"))?;
            report.read(io::BufReader::new(file), &source)?;
        }
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&report.value()).map_err(|error| error.to_string())?
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row() -> Value {
        json!({"case":"read", "target":{"model":"muse"}, "trial":0,
            "passed":true, "error":null, "duration_ms":1000,
            "metadata":{"surface":"native-bounded", "usage_convention":"everruns-disjoint-v1", "actual_cost_complete":true},
            "usage":{"input_tokens":100,"cache_read_tokens":900,"output_tokens":20,"reasoning_tokens":12,"cost_usd":0.01},
            "metrics":{"platform.model_calls":2,"platform.operations":1}, "scores":[]})
    }

    #[test]
    fn cached_input_and_reasoning_are_not_double_counted() {
        let mut report = Report::default();
        report.add(row()).unwrap();
        let value = report.value();
        let result = &value["arms"][0]["results"];
        assert_eq!(result["total_input_tokens"], 1000);
        assert_eq!(result["output_tokens"], 20);
        assert_eq!(result["actual_cost_complete"], true);
    }

    #[test]
    fn failures_and_missing_cost_remain_in_denominator() {
        let mut report = Report::default();
        report.add(row()).unwrap();
        let mut failed = row();
        failed["passed"] = json!(false);
        failed["error"] = json!("provider unavailable");
        failed["error_kind"] = json!("Infra");
        failed["metadata"]["actual_cost_complete"] = json!(false);
        failed["usage"] = json!({});
        report.add(failed).unwrap();
        let value = report.value();
        let result = &value["arms"][0]["results"];
        assert_eq!(result["trials"], 2);
        assert_eq!(result["passed"], 1);
        assert_eq!(result["infra_errors"], 1);
        assert_eq!(result["mean_total_input_tokens"], 500.0);
        assert_eq!(result["actual_cost_complete"], false);
        assert_eq!(result["failures"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn different_models_and_surfaces_are_not_pooled() {
        let mut report = Report::default();
        report.add(row()).unwrap();
        let mut eager = row();
        eager["metadata"]["surface"] = json!("native-eager");
        report.add(eager).unwrap();
        let mut other_model = row();
        other_model["target"]["model"] = json!("luna");
        report.add(other_model).unwrap();
        assert_eq!(report.value()["arms"].as_array().unwrap().len(), 3);
    }

    #[test]
    fn unknown_usage_convention_is_an_error() {
        let mut value = row();
        value["metadata"]["usage_convention"] = json!("inclusive");
        assert!(
            Report::default()
                .add(value)
                .unwrap_err()
                .contains("usage convention")
        );
    }
}
