// Domain event listeners (startup phase 4).
//
// Decision: built-in listeners register in a fixed order and embedder listeners
//   are appended last. Optional listeners (run summaries, sandboxes, metrics,
//   observers, Braintrust) are added only when their feature is configured, so
//   a disabled feature adds no listener at all. Notifications are always on.

use crate::api;
use crate::auth;
use crate::channels::a2a::A2aPushListener;
use crate::storage::{EncryptionService, StorageBackend};
use crate::{domains, listeners};
use everruns_core::EventListener;
use everruns_core::host::HostComposition;
use everruns_core::host::observability::{BraintrustListener, OtelEventListener};
use std::sync::Arc;

pub(super) struct ListenerDeps {
    pub db: Arc<StorageBackend>,
    pub encryption: Option<Arc<EncryptionService>>,
    pub host_composition: Arc<HostComposition>,
    pub auth_state: auth::AuthState,
    pub auth_config: auth::AuthConfig,
    pub observers_enabled: bool,
    pub prometheus_enabled: bool,
}

/// The listener list plus the services the later phases share with it.
pub(super) struct Listeners {
    pub event_listeners: Vec<Arc<dyn EventListener>>,
    pub budget_service: Arc<crate::domains::budgets::BudgetService>,
    pub mcp_events: Arc<domains::mcp_servers::McpEventsService>,
    pub mcp_event_triggers: Arc<crate::domains::agent_triggers::McpEventTriggers>,
    pub thread_turns: Arc<listeners::coordination::ThreadTurnListener>,
    pub session_sandbox_service:
        Option<Arc<crate::domains::session_sandbox::SessionSandboxService>>,
    pub notification_service: Arc<crate::domains::notifications::NotificationService>,
    /// Wakes the observer scoring worker, which starts once the LLM stack exists.
    pub observer_wake: Option<Arc<tokio::sync::Notify>>,
}

pub(super) fn build(
    deps: ListenerDeps,
    custom_listeners: Vec<Arc<dyn EventListener>>,
) -> Listeners {
    let ListenerDeps {
        db,
        encryption,
        host_composition,
        auth_state,
        auth_config,
        ..
    } = deps;
    let budget_service = Arc::new(crate::domains::budgets::BudgetService::new(db.clone()));
    let mcp_events = domains::mcp_servers::McpEventsService::shared(
        &db,
        &encryption,
        &host_composition,
        &auth_state,
    );
    let mcp_event_triggers = crate::domains::agent_triggers::McpEventTriggers::shared(
        &db,
        &encryption,
        &host_composition,
        &auth_state,
    );
    let thread_turns = Arc::new(listeners::coordination::ThreadTurnListener::new(db.clone()));
    let mut event_listeners: Vec<Arc<dyn EventListener>> = vec![
        Arc::new(OtelEventListener::new()),
        Arc::new(domains::usage::UsageTrackingListener::new(db.clone())),
        budget_service.clone(),
        // Approvals outlive their session: knowledge/execution/soft-approval.md.
        Arc::new(domains::audit_logs::ApprovalAuditListener::new(db.clone())),
        mcp_events.listener(),
        A2aPushListener::shared(&db, &encryption, &host_composition, &auth_state),
        Arc::new(listeners::TurnLatencyListener::new()),
        thread_turns.clone(),
    ];
    // Run summaries (EVE-867). Registered only when a utility LLM is
    // configured, so the OSS default adds no listener at all rather than one
    // that wakes on every terminal turn to do nothing.
    let run_summary_service =
        listeners::RunSummaryService::new(db.clone(), Some(host_composition.utility_llm_service()));
    if run_summary_service.is_enabled() {
        event_listeners.push(Arc::new(listeners::run_summary::RunSummaryListener::new(
            run_summary_service,
        )));
    }
    let session_sandbox_service: Option<
        Arc<crate::domains::session_sandbox::SessionSandboxService>,
    > = {
        let database = db.database();
        match &encryption {
            Some(enc) => {
                let storage_store: Arc<dyn everruns_core::session_services::SessionStorageStore> =
                    Arc::new(crate::storage::create_db_session_storage_store(
                        database.clone(),
                        enc.as_ref().clone(),
                    ));
                let connection_resolver = Some(super::build_connection_resolver(
                    &db,
                    enc,
                    &auth_config,
                    host_composition.egress_service(),
                ));
                let service =
                    Arc::new(crate::domains::session_sandbox::SessionSandboxService::new(
                        db.clone(),
                        storage_store,
                        connection_resolver,
                    ));
                event_listeners.push(Arc::new(
                    crate::domains::session_sandbox::SessionSandboxEventListener::new(
                        service.clone(),
                    ),
                ));
                Some(service)
            }
            None => {
                tracing::warn!(
                    "encryption is not configured; managed Sandbox lifecycle is disabled"
                );
                None
            }
        }
    };
    let notification_service = Arc::new(crate::domains::notifications::NotificationService::new(
        db.clone(),
    ));
    event_listeners.push(Arc::new(
        crate::domains::notifications::NotificationEventListener::new(notification_service.clone()),
    ));

    if deps.prometheus_enabled {
        event_listeners
            .push(Arc::new(api::prometheus::PrometheusMetricsListener) as Arc<dyn EventListener>);
    }

    // Observers: tap turn.completed to enqueue scoring (the listener needs
    // only db + wake). The background worker — which may call an LLM judge —
    // is spawned later, once the driver registry and provider resolver exist.
    // See knowledge/evaluation/online-evals.md.
    let observer_wake = if deps.observers_enabled {
        let observer_wake = Arc::new(tokio::sync::Notify::new());
        let observer_listener: Arc<dyn EventListener> =
            Arc::new(crate::domains::observers::ObserverMatchListener::new(
                db.clone(),
                observer_wake.clone(),
            ));
        event_listeners.push(observer_listener);
        Some(observer_wake)
    } else {
        None
    };

    if let Some(braintrust_listener) = BraintrustListener::from_env() {
        tracing::info!("Braintrust integration enabled");
        event_listeners.push(Arc::new(braintrust_listener));
    }

    // Append custom event listeners
    event_listeners.extend(custom_listeners);

    Listeners {
        event_listeners,
        budget_service,
        mcp_events,
        mcp_event_triggers,
        thread_turns,
        session_sandbox_service,
        notification_service,
        observer_wake,
    }
}
