//! Private composition of application-owned services into the shared runtime.
use everruns_core::host::HostBackends;
#[cfg(any(
    feature = "sql-database",
    feature = "knowledge",
    feature = "slack-actions"
))]
use std::sync::Arc;

#[derive(Clone, Default)]
pub(crate) struct NativeServices {
    pub connections: crate::services::Connections,
    #[cfg(feature = "sql-database")]
    pub database: Option<crate::database::SqlDatabase>,
    #[cfg(feature = "knowledge")]
    pub knowledge: Option<crate::knowledge::Knowledge>,
    #[cfg(feature = "slack-actions")]
    pub slack: Option<crate::slack::SlackActions>,
}
impl NativeServices {
    pub fn apply(&self, mut backends: HostBackends) -> HostBackends {
        if !self.connections.0.is_empty() {
            backends.connection_resolver = Some(
                self.connections
                    .resolver(backends.connection_resolver.clone()),
            );
        }
        #[cfg(feature = "knowledge")]
        if let Some(knowledge) = &self.knowledge {
            let bound = knowledge.bind(backends.storage_store.clone());
            backends.extensions.hooks.knowledge = Some(bound);
            let files = knowledge.mount();
            backends
                .file_mounts
                .push(("/knowledge".into(), files.clone(), "/".into()));
            backends
                .file_mounts
                .push(("/knowledge-text".into(), files, "/".into()));
        }
        #[cfg(any(
            feature = "sql-database",
            feature = "knowledge",
            feature = "slack-actions"
        ))]
        {
            let previous = backends.tool_context_extensions_factory.clone();
            let services = self.clone();
            #[cfg(feature = "knowledge")]
            let storage = backends.storage_store.clone();
            backends.tool_context_extensions_factory = Some(Arc::new(move |org, session| {
                let mut extensions = previous
                    .as_ref()
                    .map(|factory| factory(org, session))
                    .unwrap_or_default();
                #[cfg(feature = "sql-database")]
                if let Some(database) = &services.database {
                    extensions.insert(Arc::new(
                        everruns_contracts::session_sqldb::SessionSqlDbStoreExt(
                            database.store.clone(),
                        ),
                    ));
                }
                #[cfg(feature = "knowledge")]
                if let Some(knowledge) = &services.knowledge {
                    extensions.insert(Arc::new(
                        everruns_contracts::knowledge::KnowledgeRuntimeExt(
                            knowledge.bind(storage.clone()),
                        ),
                    ));
                }
                #[cfg(feature = "slack-actions")]
                if let Some(slack) = &services.slack {
                    extensions.insert(Arc::new(
                        everruns_contracts::slack_action::SlackActionInvokerExt(Arc::new(
                            slack.clone(),
                        )),
                    ));
                }
                extensions
            }));
        }
        backends
    }
}
