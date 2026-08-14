use anyhow::Result;
use async_trait::async_trait;
use everruns_core::message_filter::MessageQuery;
use everruns_provider::typed_id::SessionId;

use super::StorageBackend;
use super::memory::InMemoryDatabase;
use super::models::*;
use super::reporting::models::ReportingOutboxRow;
use super::repositories::Database;

/// Shared hard cap for otherwise-unbounded message history reads.
pub const MESSAGE_SAFETY_LIMIT: usize = 2_000;

/// Shared storage contract for repository operations that must stay identical
/// across PostgreSQL and in-memory backends.
#[async_trait]
pub trait Repository: Send + Sync {
    async fn create_principal(&self, input: CreatePrincipalRow) -> Result<PrincipalRow>;

    async fn create_session(&self, input: CreateSessionRow) -> Result<SessionRow>;
    async fn create_event(&self, input: CreateEventRow) -> Result<EventRow>;
    async fn list_message_events_limited(
        &self,
        session_id: SessionId,
        limit: Option<i32>,
    ) -> Result<Vec<EventRow>>;
    async fn list_message_events_filtered(&self, query: &MessageQuery) -> Result<Vec<EventRow>>;

    async fn create_provider(&self, org_id: i64, input: CreateProviderRow) -> Result<ProviderRow>;
    async fn list_providers(&self, org_id: i64) -> Result<Vec<ProviderRow>>;

    async fn upsert_agent_by_name(
        &self,
        org_id: i64,
        input: CreateAgentRow,
    ) -> Result<(AgentRow, bool)>;

    async fn create_budget(&self, input: CreateBudgetRow) -> Result<BudgetRow>;
    async fn create_usage_journal_entry(
        &self,
        input: CreateUsageJournalRow,
    ) -> Result<UsageJournalRow>;
    async fn create_usage_ledger_entry(
        &self,
        input: CreateUsageLedgerRow,
    ) -> Result<(UsageLedgerRow, Option<BudgetRow>)>;

    async fn list_reporting_outbox(
        &self,
        org_id: i64,
        source_type: &str,
        source_id: &str,
        reason: &str,
    ) -> Result<Vec<ReportingOutboxRow>>;
}

#[async_trait]
impl Repository for Database {
    async fn create_principal(&self, input: CreatePrincipalRow) -> Result<PrincipalRow> {
        Database::create_principal(self, input).await
    }

    async fn create_session(&self, input: CreateSessionRow) -> Result<SessionRow> {
        Database::create_session(self, input).await
    }

    async fn create_event(&self, input: CreateEventRow) -> Result<EventRow> {
        Database::create_event(self, input).await
    }

    async fn list_message_events_limited(
        &self,
        session_id: SessionId,
        limit: Option<i32>,
    ) -> Result<Vec<EventRow>> {
        Database::list_message_events_limited(self, session_id, limit).await
    }

    async fn list_message_events_filtered(&self, query: &MessageQuery) -> Result<Vec<EventRow>> {
        Database::list_message_events_filtered(self, query).await
    }

    async fn create_provider(&self, org_id: i64, input: CreateProviderRow) -> Result<ProviderRow> {
        Database::create_provider(self, org_id, input).await
    }

    async fn list_providers(&self, org_id: i64) -> Result<Vec<ProviderRow>> {
        Database::list_providers(self, org_id).await
    }

    async fn upsert_agent_by_name(
        &self,
        org_id: i64,
        input: CreateAgentRow,
    ) -> Result<(AgentRow, bool)> {
        Database::upsert_agent_by_name(self, org_id, input).await
    }

    async fn create_budget(&self, input: CreateBudgetRow) -> Result<BudgetRow> {
        Database::create_budget(self, input).await
    }

    async fn create_usage_journal_entry(
        &self,
        input: CreateUsageJournalRow,
    ) -> Result<UsageJournalRow> {
        Database::create_usage_journal_entry(self, input).await
    }

    async fn create_usage_ledger_entry(
        &self,
        input: CreateUsageLedgerRow,
    ) -> Result<(UsageLedgerRow, Option<BudgetRow>)> {
        Database::create_usage_ledger_entry(self, input).await
    }

    async fn list_reporting_outbox(
        &self,
        org_id: i64,
        source_type: &str,
        source_id: &str,
        reason: &str,
    ) -> Result<Vec<ReportingOutboxRow>> {
        Database::list_reporting_outbox(self, org_id, source_type, source_id, reason).await
    }
}

#[async_trait]
impl Repository for InMemoryDatabase {
    async fn create_principal(&self, input: CreatePrincipalRow) -> Result<PrincipalRow> {
        InMemoryDatabase::create_principal(self, input).await
    }

    async fn create_session(&self, input: CreateSessionRow) -> Result<SessionRow> {
        InMemoryDatabase::create_session(self, input).await
    }

    async fn create_event(&self, input: CreateEventRow) -> Result<EventRow> {
        InMemoryDatabase::create_event(self, input).await
    }

    async fn list_message_events_limited(
        &self,
        session_id: SessionId,
        limit: Option<i32>,
    ) -> Result<Vec<EventRow>> {
        InMemoryDatabase::list_message_events_limited(self, session_id, limit).await
    }

    async fn list_message_events_filtered(&self, query: &MessageQuery) -> Result<Vec<EventRow>> {
        InMemoryDatabase::list_message_events_filtered(self, query).await
    }

    async fn create_provider(&self, org_id: i64, input: CreateProviderRow) -> Result<ProviderRow> {
        InMemoryDatabase::create_provider(self, org_id, input).await
    }

    async fn list_providers(&self, org_id: i64) -> Result<Vec<ProviderRow>> {
        InMemoryDatabase::list_providers(self, org_id).await
    }

    async fn upsert_agent_by_name(
        &self,
        org_id: i64,
        input: CreateAgentRow,
    ) -> Result<(AgentRow, bool)> {
        InMemoryDatabase::upsert_agent_by_name(self, org_id, input).await
    }

    async fn create_budget(&self, input: CreateBudgetRow) -> Result<BudgetRow> {
        InMemoryDatabase::create_budget(self, input).await
    }

    async fn create_usage_journal_entry(
        &self,
        input: CreateUsageJournalRow,
    ) -> Result<UsageJournalRow> {
        InMemoryDatabase::create_usage_journal_entry(self, input).await
    }

    async fn create_usage_ledger_entry(
        &self,
        input: CreateUsageLedgerRow,
    ) -> Result<(UsageLedgerRow, Option<BudgetRow>)> {
        InMemoryDatabase::create_usage_ledger_entry(self, input).await
    }

    async fn list_reporting_outbox(
        &self,
        org_id: i64,
        source_type: &str,
        source_id: &str,
        reason: &str,
    ) -> Result<Vec<ReportingOutboxRow>> {
        InMemoryDatabase::list_reporting_outbox(self, org_id, source_type, source_id, reason).await
    }
}

#[async_trait]
impl Repository for StorageBackend {
    async fn create_principal(&self, input: CreatePrincipalRow) -> Result<PrincipalRow> {
        StorageBackend::create_principal(self, input).await
    }

    async fn create_session(&self, input: CreateSessionRow) -> Result<SessionRow> {
        StorageBackend::create_session(self, input).await
    }

    async fn create_event(&self, input: CreateEventRow) -> Result<EventRow> {
        StorageBackend::create_event(self, input).await
    }

    async fn list_message_events_limited(
        &self,
        session_id: SessionId,
        limit: Option<i32>,
    ) -> Result<Vec<EventRow>> {
        StorageBackend::list_message_events_limited(self, session_id, limit).await
    }

    async fn list_message_events_filtered(&self, query: &MessageQuery) -> Result<Vec<EventRow>> {
        StorageBackend::list_message_events_filtered(self, query).await
    }

    async fn create_provider(&self, org_id: i64, input: CreateProviderRow) -> Result<ProviderRow> {
        StorageBackend::create_provider(self, org_id, input).await
    }

    async fn list_providers(&self, org_id: i64) -> Result<Vec<ProviderRow>> {
        StorageBackend::list_providers(self, org_id).await
    }

    async fn upsert_agent_by_name(
        &self,
        org_id: i64,
        input: CreateAgentRow,
    ) -> Result<(AgentRow, bool)> {
        StorageBackend::upsert_agent_by_name(self, org_id, input).await
    }

    async fn create_budget(&self, input: CreateBudgetRow) -> Result<BudgetRow> {
        StorageBackend::create_budget(self, input).await
    }

    async fn create_usage_journal_entry(
        &self,
        input: CreateUsageJournalRow,
    ) -> Result<UsageJournalRow> {
        StorageBackend::create_usage_journal_entry(self, input).await
    }

    async fn create_usage_ledger_entry(
        &self,
        input: CreateUsageLedgerRow,
    ) -> Result<(UsageLedgerRow, Option<BudgetRow>)> {
        StorageBackend::create_usage_ledger_entry(self, input).await
    }

    async fn list_reporting_outbox(
        &self,
        org_id: i64,
        source_type: &str,
        source_id: &str,
        reason: &str,
    ) -> Result<Vec<ReportingOutboxRow>> {
        StorageBackend::list_reporting_outbox(self, org_id, source_type, source_id, reason).await
    }
}
