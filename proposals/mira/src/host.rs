//! Host side of the eval protocol. Spawns the server process and issues
//! `initialize` / `list` / `run` requests, handling interleaved progress
//! notifications. The `eval` CLI (`src/bin/mira.rs`) is the user-facing driver
//! built on top of this.

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::{ChildStdin, ChildStdout, Command};

use crate::protocol::{
    InitializeResult, ListResult, Notification, PROTOCOL_VERSION, Request, Response, RunParams,
    RunResult,
};

/// A spawned server process and the framed stdio channel to it.
pub struct Host {
    child: tokio::process::Child,
    stdin: ChildStdin,
    stdout: Lines<BufReader<ChildStdout>>,
    next_id: u64,
    /// Called for each progress notification (e.g. to render a live log).
    on_event: Box<dyn Fn(&Notification) + Send>,
}

impl Host {
    /// Spawn `command` as the eval server. Its stderr is inherited (build logs,
    /// tracing); only stdout carries protocol JSON.
    pub async fn spawn(mut command: Command) -> std::io::Result<Self> {
        command
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::inherit());
        let mut child = command.spawn()?;
        let stdin = child.stdin.take().expect("piped stdin");
        let stdout = child.stdout.take().expect("piped stdout");
        Ok(Self {
            child,
            stdin,
            stdout: BufReader::new(stdout).lines(),
            next_id: 0,
            on_event: Box::new(|_| {}),
        })
    }

    /// Register a callback for progress notifications.
    pub fn on_event(mut self, f: impl Fn(&Notification) + Send + 'static) -> Self {
        self.on_event = Box::new(f);
        self
    }

    pub async fn initialize(&mut self, host_name: &str) -> Result<InitializeResult, String> {
        let value = self
            .request(
                "initialize",
                serde_json::json!({ "protocol_version": PROTOCOL_VERSION, "host": host_name }),
            )
            .await?;
        serde_json::from_value(value).map_err(|e| e.to_string())
    }

    pub async fn list(&mut self) -> Result<ListResult, String> {
        let value = self.request("list", serde_json::Value::Null).await?;
        serde_json::from_value(value).map_err(|e| e.to_string())
    }

    pub async fn run(
        &mut self,
        eval: &str,
        sample: &str,
        model: &str,
    ) -> Result<RunResult, String> {
        let params = RunParams {
            eval: eval.into(),
            sample: sample.into(),
            model: model.into(),
        };
        let value = self
            .request("run", serde_json::to_value(params).unwrap())
            .await?;
        serde_json::from_value(value).map_err(|e| e.to_string())
    }

    /// Close stdin and wait for the server to exit.
    pub async fn shutdown(mut self) -> std::io::Result<()> {
        drop(self.stdin);
        self.child.wait().await.map(|_| ())
    }

    /// Send one request and read until its correlated response arrives,
    /// dispatching any notifications seen along the way.
    async fn request(
        &mut self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        self.next_id += 1;
        let id = self.next_id;
        let request = Request {
            id,
            method: method.into(),
            params,
        };
        let mut line = serde_json::to_vec(&request).map_err(|e| e.to_string())?;
        line.push(b'\n');
        self.stdin
            .write_all(&line)
            .await
            .map_err(|e| e.to_string())?;
        self.stdin.flush().await.map_err(|e| e.to_string())?;

        loop {
            let line = self
                .stdout
                .next_line()
                .await
                .map_err(|e| e.to_string())?
                .ok_or_else(|| "server closed the connection".to_string())?;
            if line.trim().is_empty() {
                continue;
            }
            // A response carries `id`; a notification carries `method` only.
            if let Ok(response) = serde_json::from_str::<Response>(&line) {
                if response.id != id {
                    continue;
                }
                return match (response.result, response.error) {
                    (Some(result), _) => Ok(result),
                    (None, Some(err)) => Err(err.message),
                    (None, None) => Err("empty response".into()),
                };
            }
            if let Ok(notification) = serde_json::from_str::<Notification>(&line) {
                (self.on_event)(&notification);
            }
        }
    }
}
