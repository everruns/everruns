//! Applying the chart and showing the rollout: Helm runs with `--wait` while
//! a spinner shows each workload's ready count, polled from kubectl. Helm's
//! own output is shown only when it fails.

use std::io::Write;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde_json::Value;

use super::cluster::Target;
use crate::ui;

/// `helm upgrade --install` with the values on stdin, so the database URL and
/// provider keys never touch this machine's disk.
pub(super) fn helm_apply(
    target: &Target,
    chart: &str,
    version: Option<&str>,
    values: &Value,
    set: &[String],
    files: &[PathBuf],
    timeout: &str,
) -> Result<()> {
    let mut cmd = target.helm();
    cmd.args(["upgrade", "--install", &target.release, chart]);
    if let Some(v) = version {
        cmd.args(["--version", v]);
    }
    cmd.args([
        "--create-namespace",
        "--wait",
        "--timeout",
        timeout,
        "--values",
        "-",
    ]);
    for f in files {
        cmd.arg("--values").arg(f);
    }
    for s in set {
        cmd.args(["--set", s]);
    }
    // Helm's own output (release notes) is noise next to the live pod
    // counts; it is shown only when helm fails.
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("Failed to run helm")?;
    child
        .stdin
        .take()
        .context("helm has no stdin")?
        .write_all(serde_json::to_string(values)?.as_bytes())
        .context("Failed to pass the values to helm")?;

    let spinner = ui::Spinner::start(match version {
        Some(v) => format!("Applying chart {v} and waiting for the pods"),
        None => "Applying the chart and waiting for the pods".to_string(),
    });
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let watcher = {
        let stop = stop.clone();
        let target = target.clone();
        let bar = spinner.handle();
        let base = bar.message();
        std::thread::spawn(move || {
            let mut last = Vec::new();
            while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                if let Some(now) = workloads(&target) {
                    bar.set_message(format!("{base}  {}", ui::dim(progress_line(&now))));
                    last = now;
                }
                for _ in 0..20 {
                    if stop.load(std::sync::atomic::Ordering::Relaxed) {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
            }
            last
        })
    };
    let output = child.wait_with_output().context("Failed to run helm")?;
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    let _ = watcher.join();
    if !output.status.success() {
        spinner.failed("helm upgrade --install failed");
        eprint!("{}", String::from_utf8_lossy(&output.stdout));
        eprint!("{}", String::from_utf8_lossy(&output.stderr));
        bail!(
            "`helm upgrade --install` failed; see the output above. `everruns k8s status` and \
             `everruns k8s logs server` show what the pods are doing"
        );
    }
    spinner.done("Chart applied");
    for w in workloads(target).unwrap_or_default() {
        ui::ok(format!(
            "{:<12} {}",
            w.name,
            ui::dim(format!("{}/{} ready", w.ready, w.want))
        ));
    }
    Ok(())
}

#[derive(Debug, PartialEq)]
struct Workload {
    name: String,
    ready: u64,
    want: u64,
}

/// The install's Deployments and StatefulSets with their ready counts, or
/// `None` when kubectl does not answer (the namespace may not exist yet).
fn workloads(target: &Target) -> Option<Vec<Workload>> {
    let out = target
        .kubectl()
        .args([
            "get",
            "deployments,statefulsets",
            "--selector",
            &target.selector(None),
            "--output",
            "json",
        ])
        .stderr(Stdio::null())
        .output()
        .ok()
        .filter(|o| o.status.success())?;
    let list: Value = serde_json::from_slice(&out.stdout).ok()?;
    Some(parse_workloads(&list))
}

fn parse_workloads(list: &Value) -> Vec<Workload> {
    let mut found: Vec<Workload> = list["items"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|item| Workload {
            name: item["metadata"]["labels"]["app.kubernetes.io/component"]
                .as_str()
                .or_else(|| item["metadata"]["name"].as_str())
                .unwrap_or("?")
                .to_string(),
            ready: item["status"]["readyReplicas"].as_u64().unwrap_or(0),
            want: item["spec"]["replicas"].as_u64().unwrap_or(1),
        })
        .collect();
    found.sort_by(|a, b| a.name.cmp(&b.name));
    found
}

/// `server 1/2 · worker 0/1`, waiting ones first.
fn progress_line(workloads: &[Workload]) -> String {
    let mut sorted: Vec<&Workload> = workloads.iter().collect();
    sorted.sort_by_key(|w| w.ready >= w.want);
    sorted
        .iter()
        .map(|w| format!("{} {}/{}", w.name, w.ready, w.want))
        .collect::<Vec<_>>()
        .join(" · ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workloads_read_ready_counts_by_component() {
        let list = serde_json::json!({ "items": [
            { "metadata": { "name": "everruns-worker",
                "labels": { "app.kubernetes.io/component": "worker" } },
              "spec": { "replicas": 1 }, "status": {} },
            { "metadata": { "name": "everruns-server",
                "labels": { "app.kubernetes.io/component": "server" } },
              "spec": { "replicas": 2 }, "status": { "readyReplicas": 2 } },
        ]});
        let w = parse_workloads(&list);
        assert_eq!(
            w[0],
            Workload {
                name: "server".into(),
                ready: 2,
                want: 2
            }
        );
        assert_eq!(
            w[1],
            Workload {
                name: "worker".into(),
                ready: 0,
                want: 1
            }
        );
        // Waiting ones lead, so the spinner shows what is holding things up.
        assert_eq!(progress_line(&w), "worker 0/1 · server 2/2");
    }
}
