//! Private, immutable checkpoint and effect records. Never use agent-writable KV.
use async_trait::async_trait;
use everruns_contracts::{
    error::{AgentLoopError, Result},
    runtime::{
        durability::{
            CodeExecutionJournal, DurableToolCallStatus, DurableToolResultStore,
            ToolCallClaimResult,
        },
        tool_context::ToolContext,
    },
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
use uuid::Uuid;

pub(super) fn digest(bytes: impl AsRef<[u8]>) -> String {
    hex::encode(Sha256::digest(bytes.as_ref()))
}

pub(super) fn resolve(
    ctx: &ToolContext,
    directory: Option<&Path>,
) -> std::result::Result<CodeExecutionJournal, String> {
    if let Some(journal) = ctx.extension::<CodeExecutionJournal>() {
        return Ok((*journal).clone());
    }
    let directory = directory.ok_or("Python tool calls and resume require a private journal: configure EVERRUNS_MONTY_STATE_DIR or supply CodeExecutionJournal")?;
    if !directory.is_absolute() {
        return Err("Monty state directory must be an absolute, host-owned path".into());
    }
    let turn = ctx
        .event_context
        .as_ref()
        .and_then(|c| c.turn_id)
        .map(|v| v.to_string())
        .unwrap_or_else(|| "standalone".into());
    Ok(CodeExecutionJournal {
        store: Arc::new(FileJournal {
            directory: directory.to_owned(),
        }),
        namespace: format!("code:{}:{turn}", ctx.session_id),
    })
}

pub(super) async fn load(
    journal: &CodeExecutionJournal,
    key: &str,
) -> std::result::Result<Option<Value>, String> {
    match journal.store.get_tool_call_status(&journal.namespace, key).await.map_err(|_| "Python journal unavailable")? {
        None => Ok(None),
        Some(DurableToolCallStatus::Settled { result_json }) => Ok(Some(result_json)),
        _ => Err("Python execution was interrupted with an unsettled record; its effects must be reconciled before retrying".into()),
    }
}

pub(super) async fn save(
    journal: &CodeExecutionJournal,
    key: &str,
    value: Value,
) -> std::result::Result<(), String> {
    let fp = digest(serde_json::to_vec(&value).map_err(|_| "invalid checkpoint")?);
    match journal
        .store
        .try_claim_tool_call(&journal.namespace, key, "python.checkpoint", &fp)
        .await
        .map_err(|_| "Python journal unavailable")?
    {
        ToolCallClaimResult::Claimed { claim_token } => {
            if !journal
                .store
                .settle_tool_call(&journal.namespace, key, value, "settled", claim_token)
                .await
                .map_err(|_| "Python checkpoint could not be saved")?
            {
                return Err("Python checkpoint ownership lost".into());
            }
            Ok(())
        }
        ToolCallClaimResult::AlreadySettled {
            args_fingerprint, ..
        } if args_fingerprint == fp => Ok(()),
        _ => Err("Python checkpoint conflicts with an earlier execution; refusing replay".into()),
    }
}

/// Persistent journal for Framework/agentd hosts without a platform result
/// store. The operator owns this directory; it must never be a session mount.
/// Claim files are immutable and effects with no result are never retried.
struct FileJournal {
    directory: PathBuf,
}
impl FileJournal {
    fn path(&self, namespace: &str, key: &str, suffix: &str) -> PathBuf {
        self.directory.join(format!(
            "{}.{suffix}",
            digest(format!("{namespace}\0{key}"))
        ))
    }
    async fn read(&self, path: &Path) -> Result<Option<Value>> {
        use tokio::io::AsyncReadExt;
        const LIMIT: usize = 4 * 1024 * 1024;
        let mut options = tokio::fs::OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        options.custom_flags(libc::O_NOFOLLOW);
        let file = match options.open(path).await {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err(AgentLoopError::tool("private Python journal unavailable")),
        };
        let mut bytes = Vec::new();
        file.take((LIMIT + 1) as u64)
            .read_to_end(&mut bytes)
            .await
            .map_err(|_| AgentLoopError::tool("private Python journal unavailable"))?;
        if bytes.len() > LIMIT {
            return Err(AgentLoopError::tool(
                "private Python journal record too large",
            ));
        }
        serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|_| AgentLoopError::tool("invalid private Python journal record"))
    }
    async fn create(&self, path: &Path, value: &Value) -> Result<bool> {
        use tokio::io::AsyncWriteExt;
        let mut options = tokio::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        options.mode(0o600);
        let mut file = match options.open(path).await {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => return Ok(false),
            Err(_) => {
                return Err(AgentLoopError::tool(
                    "cannot create private Python journal record",
                ));
            }
        };
        file.write_all(
            &serde_json::to_vec(value)
                .map_err(|_| AgentLoopError::tool("invalid Python journal value"))?,
        )
        .await
        .map_err(|_| AgentLoopError::tool("cannot write Python journal"))?;
        file.sync_all()
            .await
            .map_err(|_| AgentLoopError::tool("cannot sync Python journal"))?;
        // Persist the directory entry before an external effect can run.
        let directory = tokio::fs::File::open(&self.directory)
            .await
            .map_err(|_| AgentLoopError::tool("cannot open Python journal directory"))?;
        directory
            .sync_all()
            .await
            .map_err(|_| AgentLoopError::tool("cannot sync Python journal directory"))?;
        Ok(true)
    }
}
#[async_trait]
impl DurableToolResultStore for FileJournal {
    async fn try_claim_tool_call(
        &self,
        turn: &str,
        key: &str,
        _name: &str,
        fingerprint: &str,
    ) -> Result<ToolCallClaimResult> {
        // No create_dir_all: parent ownership and persistence are an operator
        // decision, and silent creation under the workspace is unsafe.
        let metadata = tokio::fs::symlink_metadata(&self.directory)
            .await
            .map_err(|_| {
                AgentLoopError::tool("create the private Monty state directory before running")
            })?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(AgentLoopError::tool(
                "Monty journal must be a private directory",
            ));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if metadata.permissions().mode() & 0o077 != 0 {
                return Err(AgentLoopError::tool(
                    "Monty journal directory must have mode 0700",
                ));
            }
        }
        let token = Uuid::new_v4();
        let path = self.path(turn, key, "claim");
        if self
            .create(&path, &json!({"token":token,"fingerprint":fingerprint}))
            .await?
        {
            return Ok(ToolCallClaimResult::Claimed { claim_token: token });
        }
        let claim = self
            .read(&path)
            .await?
            .ok_or_else(|| AgentLoopError::tool("missing Python claim"))?;
        let stored = claim["fingerprint"]
            .as_str()
            .ok_or_else(|| AgentLoopError::tool("invalid Python claim"))?
            .to_string();
        if stored != fingerprint {
            return Ok(ToolCallClaimResult::DeterminismViolation {
                stored_fingerprint: stored,
                current_fingerprint: fingerprint.into(),
            });
        }
        Ok(match self.read(&self.path(turn, key, "result")).await? {
            Some(result) => ToolCallClaimResult::AlreadySettled {
                result_json: result,
                args_fingerprint: stored,
            },
            None => ToolCallClaimResult::AlreadyRunning {
                args_fingerprint: stored,
            },
        })
    }
    async fn settle_tool_call(
        &self,
        turn: &str,
        key: &str,
        value: Value,
        status: &str,
        token: Uuid,
    ) -> Result<bool> {
        if status != "settled" {
            return Err(AgentLoopError::tool("unsupported Python journal status"));
        }
        let claim = self.read(&self.path(turn, key, "claim")).await?;
        if claim.as_ref().and_then(|c| c["token"].as_str()) != Some(token.to_string().as_str()) {
            return Ok(false);
        }
        let result = self.path(turn, key, "result");
        // Fully write + fsync before atomically publishing. Readers never see
        // a partial snapshot. Hard links cannot replace an existing result.
        let temporary = self.path(turn, key, &format!("tmp-{token}"));
        if !self.create(&temporary, &value).await? {
            return Ok(false);
        }
        let published = tokio::fs::hard_link(&temporary, &result).await;
        let _ = tokio::fs::remove_file(&temporary).await;
        match published {
            Ok(()) => {
                let directory = tokio::fs::File::open(&self.directory)
                    .await
                    .map_err(|_| AgentLoopError::tool("cannot open Python journal directory"))?;
                directory
                    .sync_all()
                    .await
                    .map_err(|_| AgentLoopError::tool("cannot sync Python journal directory"))?;
                Ok(true)
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(false),
            Err(_) => Err(AgentLoopError::tool("cannot publish Python journal result")),
        }
    }
    async fn get_tool_call_status(
        &self,
        turn: &str,
        key: &str,
    ) -> Result<Option<DurableToolCallStatus>> {
        if let Some(result_json) = self.read(&self.path(turn, key, "result")).await? {
            return Ok(Some(DurableToolCallStatus::Settled { result_json }));
        }
        Ok(self
            .read(&self.path(turn, key, "claim"))
            .await?
            .map(|_| DurableToolCallStatus::Running))
    }
}
