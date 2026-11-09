//! Agent definitions kept by the SQLite authority (see `hosting::definitions`).
use super::*;
use crate::hosting::{AgentDefinition, DefinitionStore};

pub(super) const SCHEMA: &str = "
    CREATE TABLE IF NOT EXISTS framework_agent_definitions(agent TEXT NOT NULL,revision TEXT NOT NULL,value TEXT NOT NULL,PRIMARY KEY(agent,revision));
    CREATE TABLE IF NOT EXISTS framework_agent_current(agent TEXT PRIMARY KEY,revision TEXT NOT NULL);";

fn load(tx: &rusqlite::Connection, build: &AgentBuild) -> Result<Option<AgentDefinition>> {
    tx.query_row(
        "SELECT value FROM framework_agent_definitions WHERE agent=? AND revision=?",
        params![build.agent, build.build_id],
        |row| row.get::<_, String>(0),
    )
    .optional()
    .map_err(error)?
    .map(decode)
    .transpose()
}

#[async_trait]
impl crate::hosting::AgentDefinitions for SqliteHosting {
    async fn publish_definition(&self, definition: &AgentDefinition) -> Result<()> {
        let mut db = self
            .db
            .lock()
            .map_err(|_| error("authority database lock poisoned"))?;
        let tx = db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(error)?;
        // One rule for every built-in authority: the shared store decides.
        let mut store = DefinitionStore::default();
        if let Some(kept) = load(&tx, &definition.build)? {
            store.publish(&kept)?;
        }
        store.publish(definition)?;
        tx.execute(
            "INSERT OR IGNORE INTO framework_agent_definitions VALUES (?,?,?)",
            params![
                definition.build.agent,
                definition.build.build_id,
                encode(definition)?
            ],
        )
        .map_err(error)?;
        tx.execute(
            "INSERT INTO framework_agent_current VALUES (?,?) ON CONFLICT(agent) DO UPDATE SET revision=excluded.revision",
            params![definition.build.agent, definition.build.build_id],
        )
        .map_err(error)?;
        tx.commit().map_err(error)
    }

    async fn definition(&self, build: &AgentBuild) -> Result<Option<AgentDefinition>> {
        let db = self
            .db
            .lock()
            .map_err(|_| error("authority database lock poisoned"))?;
        load(&db, build)
    }
}

/// Move a session between turns to its agent's current revision, in the
/// claim transaction, so the new turn and its retained revision agree.
pub(super) fn advance(tx: &Transaction<'_>, state: &mut Control) -> Result<()> {
    let in_turn = state.open.is_some() || !state.pending.is_empty() || !state.finalizing.is_empty();
    let Some(binding) = state.binding.as_mut() else {
        return Ok(());
    };
    let current: Option<String> = tx
        .query_row(
            "SELECT revision FROM framework_agent_current WHERE agent=?",
            [&binding.behavior.agent],
            |row| row.get(0),
        )
        .optional()
        .map_err(error)?;
    crate::hosting::definitions::advance(binding, in_turn, current.as_deref());
    Ok(())
}
