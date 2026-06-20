//! Server side of the eval protocol. Your eval program defines its evals in
//! Rust and hands them to [`serve`]; this runs the stdio loop that answers the
//! host's `initialize` / `list` / `run` requests.
//!
//! ```no_run
//! # async fn f(evals: Vec<mira::Eval>) -> std::io::Result<()> {
//! mira::serve(evals).await
//! # }
//! ```
//!
//! Keep stdout clean: only protocol JSON goes there. Logging belongs on stderr.

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use crate::eval::Eval;
use crate::protocol::{
    EvalInfo, InitializeResult, ListResult, ModelInfo, Notification, PROTOCOL_VERSION, Request,
    Response, RunParams, RunResult, SampleInfo, TranscriptSummary,
};
use crate::runner::run_cell;

/// Serve `evals` over newline-delimited JSON on stdin/stdout until EOF.
pub async fn serve(evals: Vec<Eval>) -> std::io::Result<()> {
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    let mut stdout = tokio::io::stdout();

    while let Some(line) = lines.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }
        let request: Request = match serde_json::from_str(&line) {
            Ok(req) => req,
            Err(e) => {
                // Can't correlate a malformed line to an id; report and move on.
                write_line(&mut stdout, &log_notification(format!("bad request: {e}"))).await?;
                continue;
            }
        };

        let response = dispatch(&evals, &request, &mut stdout).await;
        write_line(&mut stdout, &response).await?;
    }
    Ok(())
}

async fn dispatch(evals: &[Eval], request: &Request, stdout: &mut tokio::io::Stdout) -> Response {
    match request.method.as_str() {
        "initialize" => Response::ok(
            request.id,
            json(&InitializeResult {
                protocol_version: PROTOCOL_VERSION.into(),
                server: env!("CARGO_PKG_NAME").into(),
                evals: evals.len(),
            }),
        ),
        "list" => Response::ok(request.id, json(&list(evals))),
        "run" => {
            let params: RunParams = match serde_json::from_value(request.params.clone()) {
                Ok(p) => p,
                Err(e) => return Response::err(request.id, format!("bad run params: {e}")),
            };
            // Progress so the host can render a live spinner / log.
            let _ = write_line(
                stdout,
                &event_notification(&params.eval, &params.sample, &params.model, "started"),
            )
            .await;
            match run(evals, &params).await {
                Ok(result) => Response::ok(request.id, json(&result)),
                Err(e) => Response::err(request.id, e),
            }
        }
        other => Response::err(request.id, format!("unknown method: {other}")),
    }
}

fn list(evals: &[Eval]) -> ListResult {
    let evals = evals
        .iter()
        .map(|eval| EvalInfo {
            name: eval.name.clone(),
            samples: eval
                .dataset
                .samples
                .iter()
                .map(|s| SampleInfo {
                    id: s.id.clone(),
                    tags: s.tags.clone(),
                })
                .collect(),
            scorers: eval.scorers.iter().map(|s| s.name()).collect(),
            models: eval
                .models
                .iter()
                .map(|m| ModelInfo {
                    label: m.label.clone(),
                    available: !m.missing_key(),
                })
                .collect(),
            max_turns: eval.max_turns,
        })
        .collect();
    ListResult { evals }
}

async fn run(evals: &[Eval], params: &RunParams) -> Result<RunResult, String> {
    let eval = evals
        .iter()
        .find(|e| e.name == params.eval)
        .ok_or_else(|| format!("no such eval: {}", params.eval))?;
    let sample = eval
        .dataset
        .samples
        .iter()
        .find(|s| s.id == params.sample)
        .ok_or_else(|| format!("no such sample: {}/{}", params.eval, params.sample))?;
    let model = eval
        .models
        .iter()
        .find(|m| m.label == params.model)
        .ok_or_else(|| format!("no such model: {}@{}", params.eval, params.model))?;

    // Don't burn time on an unrunnable cell; report it as skipped.
    if model.missing_key() {
        return Ok(RunResult {
            eval: params.eval.clone(),
            sample: params.sample.clone(),
            model: params.model.clone(),
            passed: false,
            aggregate: 0.0,
            scores: Vec::new(),
            transcript: TranscriptSummary::default(),
            skipped: true,
        });
    }

    let outcome = run_cell(eval, sample, model).await;
    Ok(RunResult {
        eval: outcome.eval,
        sample: outcome.sample_id,
        model: outcome.model,
        passed: outcome.passed,
        aggregate: outcome.aggregate,
        scores: outcome.scores,
        transcript: TranscriptSummary {
            final_response: outcome.transcript.final_response,
            iterations: outcome.transcript.iterations,
            tool_calls_count: outcome.transcript.tool_calls_count,
            tool_calls: outcome.transcript.tool_calls,
            usage: outcome.transcript.usage,
            error: outcome.transcript.error,
        },
        skipped: false,
    })
}

fn json<T: serde::Serialize>(value: &T) -> serde_json::Value {
    serde_json::to_value(value).unwrap_or(serde_json::Value::Null)
}

fn log_notification(message: String) -> Notification {
    Notification {
        method: "log".into(),
        params: serde_json::json!({ "message": message }),
    }
}

fn event_notification(eval: &str, sample: &str, model: &str, kind: &str) -> Notification {
    Notification {
        method: "event".into(),
        params: serde_json::json!({
            "eval": eval, "sample": sample, "model": model, "kind": kind,
        }),
    }
}

async fn write_line<T: serde::Serialize>(
    stdout: &mut tokio::io::Stdout,
    value: &T,
) -> std::io::Result<()> {
    let mut buf = serde_json::to_vec(value).unwrap_or_default();
    buf.push(b'\n');
    stdout.write_all(&buf).await?;
    stdout.flush().await
}
