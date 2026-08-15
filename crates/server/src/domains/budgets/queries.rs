use crate::domains::budgets::BudgetService;
use crate::domains::common::CommandError;
use crate::storage::models::{BudgetLedgerRow, BudgetRow};
use everruns_platform::{Budget, LedgerEntry};
use everruns_provider::typed_id::BudgetId;

pub fn parse_budget_id(input: &str) -> Result<uuid::Uuid, CommandError> {
    if let Ok(id) = BudgetId::parse(input) {
        Ok(id.uuid())
    } else if let Ok(id) = uuid::Uuid::parse_str(input) {
        Ok(id)
    } else {
        Err(CommandError::bad_request("Invalid budget ID format"))
    }
}

pub fn row_to_budget(row: &BudgetRow) -> Budget {
    BudgetService::row_to_budget(row)
}

pub fn row_to_ledger_entry(row: &BudgetLedgerRow) -> LedgerEntry {
    BudgetService::row_to_ledger_entry(row)
}
