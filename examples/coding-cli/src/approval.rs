// Approval gate for destructive operations.
// Decision: writes/deletes flow through SessionFileSystem decorators; bash
// goes through the BashTool directly. Both await an explicit yes from the
// human via a oneshot channel. Read-only ops (read_file, list_directory,
// grep) run free. The TUI installs an interactive gate, the `--print`
// one-shot mode installs auto-approve. `--yes` overrides interactive.

use async_trait::async_trait;
use everruns_runtime::FileApprovalGate;
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot};

#[derive(Debug, Clone)]
pub enum ApprovalRequest {
    Bash {
        command: String,
    },
    FileWrite {
        path: String,
        /// Existing content if the file already exists (for diff display).
        before: Option<String>,
        after: String,
    },
    FileDelete {
        path: String,
        recursive: bool,
    },
}

impl ApprovalRequest {
    pub fn headline(&self) -> String {
        match self {
            Self::Bash { command } => format!("run bash: {}", first_line(command, 200)),
            Self::FileWrite {
                path,
                before,
                after,
            } => {
                let verb = if before.is_some() { "edit" } else { "create" };
                format!("{verb} {path} ({} bytes)", after.len())
            }
            Self::FileDelete { path, recursive } => {
                let r = if *recursive { " (recursive)" } else { "" };
                format!("delete {path}{r}")
            }
        }
    }

    /// Multi-line body shown above the prompt (diff, command, etc.).
    pub fn detail(&self) -> String {
        match self {
            Self::Bash { command } => command.clone(),
            Self::FileWrite {
                before,
                after,
                path,
            } => match before {
                Some(b) => crate::diff::unified(b, after, path, 3),
                None => format!("(new file, {} bytes)", after.len()),
            },
            Self::FileDelete { .. } => String::new(),
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

#[async_trait]
impl FileApprovalGate for ApprovalGate {
    async fn approve_write(&self, path: &str, before: Option<String>, after: &str) -> bool {
        self.approve(ApprovalRequest::FileWrite {
            path: path.to_string(),
            before,
            after: after.to_string(),
        })
        .await
    }

    async fn approve_delete(&self, path: &str, recursive: bool) -> bool {
        self.approve(ApprovalRequest::FileDelete {
            path: path.to_string(),
            recursive,
        })
        .await
    }
}
