//! Sessions an older serve kept in the legacy local store under
//! `data_dir/everruns`, moved to the native authority on first access
//! (EVE-1298).
use super::*;

impl Host {
    /// The authority's status of `session`, first moving a session that a
    /// release before the native authority wrote under `data_dir/everruns`.
    ///
    /// EVE-1298: the move is lazy (on first access) and idempotent, so a
    /// restart or a concurrent request mid-move finishes the same cutover.
    /// A session the authority cannot take is an error, never a silent
    /// fallback to the legacy store or a 404.
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
        let entry = self
            .app
            .agent(&row.agent)
            .ok_or_else(|| {
                anyhow!(
                    "session {id} ran agent `{}`, which this build lacks",
                    row.agent
                )
            })?
            .clone();
        let agent = self.build_agent_with(&entry, None, Some(legacy), None)?;
        let build = self.legacy_build(&entry.name);
        match self
            .engine
            .cut_over_legacy_session(session, &agent, &build)
            .await
        {
            Ok(_) => {}
            // No legacy history: the catalog row alone is not a session.
            Err(everruns::AgentLoopError::SessionNotFound(_)) => {}
            // A concurrent request may have finished the same cutover.
            Err(error) => {
                if self.engine.session_status(session).await.is_err() {
                    return Err(anyhow!(
                        "session {id} could not move to the session authority: {error}"
                    ));
                }
            }
        }
        Ok(self.engine.session_status(session).await?)
    }

    /// The local store releases before serve's native authority kept
    /// sessions in, when this data directory has one.
    pub(super) fn legacy_store(&self) -> Option<LocalConfig> {
        self.native_authority.get()?;
        let dir = self.data_dir.as_ref()?.join("everruns");
        dir.is_dir().then(|| LocalConfig::new(dir))
    }

    /// The build moved pre-authority sessions run on: the agent of this build
    /// over the shared directory workspace those sessions used, so their files
    /// and file tools carry over. New sessions never use it.
    pub(super) fn legacy_build(&self, agent: &str) -> everruns::hosting::AgentBuild {
        everruns::hosting::AgentBuild::new(agent, format!("{}:pre-authority", self.build_id))
    }
}
