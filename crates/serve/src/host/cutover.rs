//! Sessions an older serve kept in the legacy local store under
//! `data_dir/everruns`, moved to the native authority on first access
//! (EVE-1298).
//!
//! Decisions:
//! - A session moves when it is first read, not at startup, so a stale legacy
//!   lease or a broken session delays only that session.
//! - The legacy subagent tool created child sessions with no catalog row and
//!   no stored parent link. A child is found from the shared legacy journal:
//!   its `session.started` falls inside the parent's open `ask_<sub>` call
//!   (between that call's `tool.started` and `tool.completed`), and its first
//!   input is that call's `task`. Children move before their parent, so a
//!   child that cannot move leaves the parent in the legacy store and the
//!   next read retries both.
use std::collections::HashMap;
use std::io::BufRead;

use super::*;

/// Legacy children of each parent session, in journal order.
pub(super) type LegacyChildren = HashMap<String, Vec<LegacyChild>>;

#[derive(Clone, Debug)]
pub(super) struct LegacyChild {
    pub session: SessionId,
    pub agent: String,
}

impl Host {
    /// The authority's status of `session`, first moving a session that a
    /// release before the native authority wrote under `data_dir/everruns`.
    ///
    /// The move is idempotent, so a restart or a concurrent request mid-move
    /// finishes the same cutover. A session the authority cannot take is an
    /// error, never a silent fallback to the legacy store or a 404.
    pub(super) async fn hosted_status(
        &self,
        id: &str,
        session: SessionId,
    ) -> crate::Result<everruns::hosting::HostingStatus> {
        match self.engine.session_status(session).await {
            Err(everruns::AgentLoopError::SessionNotFound(_)) => {}
            other => return Ok(other?),
        }
        let (Some(legacy), Some(row)) = (self.legacy_store(), self.store.session(id)?) else {
            return Ok(self.engine.session_status(session).await?);
        };
        for child in self.legacy_children(&legacy)?.get(id).into_iter().flatten() {
            self.move_legacy(&legacy, child.session, &child.agent, Some(id))
                .await?;
        }
        self.move_legacy(&legacy, session, &row.agent, None).await?;
        Ok(self.engine.session_status(session).await?)
    }

    /// Cut `session` over to the pre-authority build of `agent`, linking it
    /// to `parent` when it is a child. A child also gets the catalog row the
    /// native subagent path writes.
    async fn move_legacy(
        &self,
        legacy: &LocalConfig,
        session: SessionId,
        agent: &str,
        parent: Option<&str>,
    ) -> crate::Result {
        if self.engine.session_status(session).await.is_ok() {
            return Ok(());
        }
        let entry = self
            .app
            .agent(agent)
            .ok_or_else(|| {
                anyhow!("session {session} ran agent `{agent}`, which this build lacks")
            })?
            .clone();
        let source = self.build_agent_with(&entry, None, Some(legacy.clone()), None)?;
        let metadata = parent.map_or(Value::Null, |parent| json!({"parent_session_id": parent}));
        match self
            .engine
            .cut_over_legacy_session_with_metadata(
                session,
                &source,
                &self.legacy_build(&entry.name),
                metadata.clone(),
            )
            .await
        {
            Ok(_) => {}
            // No legacy history: the catalog row alone is not a session.
            Err(everruns::AgentLoopError::SessionNotFound(_)) => return Ok(()),
            // A concurrent request may have finished the same cutover.
            Err(error) => {
                if self.engine.session_status(session).await.is_err() {
                    return Err(anyhow!(
                        "session {session} could not move to the session authority: {error}"
                    ));
                }
            }
        }
        if parent.is_some() {
            self.store.insert_session(&SessionRow {
                id: session.to_string(),
                agent: entry.name.clone(),
                build_id: self.legacy_build(&entry.name).build_id,
                title: None,
                tags: Vec::new(),
                hints: None,
                metadata: Some(metadata),
                created_at: now(),
                updated_at: now(),
            })?;
        }
        Ok(())
    }

    /// Children of every legacy parent, read once from the legacy journal.
    /// The legacy store no longer changes, so the index stays valid.
    fn legacy_children(&self, legacy: &LocalConfig) -> crate::Result<Arc<LegacyChildren>> {
        if let Some(index) = self.legacy_children.get() {
            return Ok(index.clone());
        }
        let subs: Vec<&str> = self
            .app
            .inner
            .agents
            .iter()
            .filter(|entry| entry.sub)
            .map(|entry| entry.name.as_str())
            .collect();
        let path = legacy.data_dir().join("events.jsonl");
        let index = if subs.is_empty() || !path.is_file() {
            LegacyChildren::new()
        } else {
            index_legacy_children(std::io::BufReader::new(std::fs::File::open(path)?), &subs)?
        };
        Ok(self.legacy_children.get_or_init(|| Arc::new(index)).clone())
    }

    /// The local store releases before serve's native authority kept
    /// sessions in, when this data directory has one.
    pub(super) fn legacy_store(&self) -> Option<LocalConfig> {
        self.native_authority.get()?;
        let dir = self.data_dir.as_ref()?.join("everruns");
        dir.is_dir().then(|| LocalConfig::new(dir))
    }

    /// Register [`Host::legacy_build`] of `entry` when this data directory
    /// has a legacy store. The legacy lane bound sessions to its shared
    /// directory workspace whatever the sandbox (the microVM adapter
    /// included), so moved sessions reopen exactly that head through an
    /// [`ImportedWorkspaceFactory`].
    ///
    /// [`ImportedWorkspaceFactory`]: everruns::hosting::ImportedWorkspaceFactory
    pub(super) fn register_pre_authority(
        &self,
        entry: &AgentEntry,
        harness: Option<everruns::Harness>,
        reference: String,
    ) -> crate::Result {
        let (Some(legacy), Some(authority)) = (self.legacy_store(), self.native_authority.get())
        else {
            return Ok(());
        };
        let moved = self.build_agent_with(
            entry,
            None,
            None,
            Some(legacy.workspace_root().to_path_buf()),
        )?;
        self.engine.register_agent_with_environment(
            self.legacy_build(&entry.name),
            moved.clone(),
            harness,
            reference,
            everruns::hosting::ImportedWorkspaceFactory::new(moved, authority.clone()),
        )?;
        Ok(())
    }

    /// The build moved pre-authority sessions run on: the agent of this build
    /// over the shared directory workspace those sessions used, so their files
    /// and file tools carry over. New sessions never use it.
    pub(super) fn legacy_build(&self, agent: &str) -> everruns::hosting::AgentBuild {
        everruns::hosting::AgentBuild::new(agent, format!("{}:pre-authority", self.build_id))
    }
}

/// Match each legacy child to the `ask_<sub>` call that created it. A torn or
/// unreadable line is skipped, as the legacy journal reader does.
pub(super) fn index_legacy_children(
    journal: impl BufRead,
    subs: &[&str],
) -> crate::Result<LegacyChildren> {
    struct Call {
        parent: String,
        id: String,
        agent: String,
        task: String,
    }
    let mut open: Vec<Call> = Vec::new();
    // Sessions started inside open calls, waiting for their first input.
    let mut starting: HashMap<String, Vec<usize>> = HashMap::new();
    let mut calls: Vec<(String, String, String, String)> = Vec::new();
    let mut claimed = std::collections::HashSet::new();
    let mut children = LegacyChildren::new();
    for line in journal.lines() {
        let Ok(event) = serde_json::from_str::<Value>(&line?) else {
            continue;
        };
        let (Some(session), Some(kind)) = (event["session_id"].as_str(), event["type"].as_str())
        else {
            continue;
        };
        let data = &event["data"];
        match kind {
            "tool.started" => {
                let call = &data["tool_call"];
                let agent = call["name"]
                    .as_str()
                    .and_then(|name| name.strip_prefix("ask_"))
                    .filter(|agent| subs.contains(agent));
                if let (Some(agent), Some(id), Some(task)) = (
                    agent,
                    call["id"].as_str(),
                    call["arguments"]["task"].as_str(),
                ) {
                    open.push(Call {
                        parent: session.into(),
                        id: id.into(),
                        agent: agent.into(),
                        task: task.into(),
                    });
                }
            }
            "tool.completed" => {
                let id = data["tool_call_id"].as_str();
                open.retain(|call| call.parent != session || Some(call.id.as_str()) != id);
            }
            "session.started" if !open.is_empty() => {
                let candidates = open
                    .iter()
                    .map(|call| {
                        calls.push((
                            call.parent.clone(),
                            call.id.clone(),
                            call.agent.clone(),
                            call.task.clone(),
                        ));
                        calls.len() - 1
                    })
                    .collect();
                starting.insert(session.into(), candidates);
            }
            "input.message" => {
                let Some(candidates) = starting.remove(session) else {
                    continue;
                };
                let text = data["message"]["content"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|part| part["text"].as_str())
                    .collect::<String>();
                // Identical concurrent delegations are interchangeable; each
                // child claims the first of them.
                let Some(&call) = candidates.iter().find(|&&call| {
                    let (parent, id, _, task) = &calls[call];
                    *task == text && !claimed.contains(&(parent.clone(), id.clone()))
                }) else {
                    continue;
                };
                let Ok(child) = session.parse() else {
                    continue;
                };
                let (parent, id, agent, _) = &calls[call];
                claimed.insert((parent.clone(), id.clone()));
                children
                    .entry(parent.clone())
                    .or_default()
                    .push(LegacyChild {
                        session: child,
                        agent: agent.clone(),
                    });
            }
            _ => {}
        }
    }
    Ok(children)
}
