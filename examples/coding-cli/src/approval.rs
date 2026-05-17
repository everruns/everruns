// Approval gate for destructive tool calls.
// Decision: write/edit/bash await an explicit yes from the human via a oneshot
// channel; read/list/grep run free. The TUI installs an interactive gate, the
// `--print` one-shot mode installs auto-approve. `--yes` overrides interactive.

use std::sync::Arc;
use tokio::sync::{mpsc, oneshot};

#[derive(Debug, Clone)]
pub enum ApprovalRequest {
    Bash {
        command: String,
    },
    Write {
        path: String,
        bytes: usize,
    },
    Edit {
        path: String,
        unified_diff: String,
        replacements: usize,
    },
}

impl ApprovalRequest {
    pub fn headline(&self) -> String {
        match self {
            Self::Bash { command } => format!("run bash: {}", first_line(command, 200)),
            Self::Write { path, bytes } => format!("write {path} ({bytes} bytes)"),
            Self::Edit {
                path, replacements, ..
            } => format!("edit {path} ({replacements} replacement(s))"),
        }
    }
    pub fn detail(&self) -> String {
        match self {
            Self::Bash { command } => command.clone(),
            Self::Write { bytes, .. } => format!("(content omitted, {bytes} bytes)"),
            Self::Edit { unified_diff, .. } => unified_diff.clone(),
        }
    }
}

fn first_line(s: &str, max: usize) -> String {
    let l = s.lines().next().unwrap_or("");
    if l.len() > max {
        format!("{}…", &l[..max])
    } else {
        l.to_string()
    }
}

#[derive(Clone)]
pub enum ApprovalGate {
    Auto,
    Channel(mpsc::UnboundedSender<(ApprovalRequest, oneshot::Sender<bool>)>),
}

impl ApprovalGate {
    pub fn auto() -> Arc<Self> {
        Arc::new(Self::Auto)
    }
    pub fn channel(
        tx: mpsc::UnboundedSender<(ApprovalRequest, oneshot::Sender<bool>)>,
    ) -> Arc<Self> {
        Arc::new(Self::Channel(tx))
    }

    pub async fn approve(&self, req: ApprovalRequest) -> bool {
        match self {
            Self::Auto => true,
            Self::Channel(tx) => {
                let (otx, orx) = oneshot::channel();
                if tx.send((req, otx)).is_err() {
                    return false;
                }
                orx.await.unwrap_or(false)
            }
        }
    }
}
