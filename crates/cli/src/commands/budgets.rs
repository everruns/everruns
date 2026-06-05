// Budget management commands
//
// Wraps the SDK's `client.budgets()` surface (create/list/get/update/delete/
// top-up/ledger/check). Budgets cap spend for a subject (org, agent, session)
// in a given currency; see `specs/budgets.md`.

use crate::output::{OutputFormat, print_field, print_table_header, print_table_row};
use anyhow::{Context, Result};
use clap::Subcommand;
use everruns_sdk::{Budget, CreateBudgetRequest, Everruns, TopUpRequest, UpdateBudgetRequest};

#[derive(Subcommand)]
pub enum BudgetsCommand {
    /// Create a budget for a subject
    Create {
        /// Subject type (e.g. organization, agent, session)
        #[arg(long)]
        subject_type: String,

        /// Subject ID the budget applies to
        #[arg(long)]
        subject_id: String,

        /// Currency code (e.g. USD)
        #[arg(long, default_value = "USD")]
        currency: String,

        /// Hard spend limit
        #[arg(long)]
        limit: f64,

        /// Soft limit (warning threshold)
        #[arg(long)]
        soft_limit: Option<f64>,
    },

    /// List budgets, optionally filtered by subject
    List {
        /// Filter by subject type
        #[arg(long)]
        subject_type: Option<String>,

        /// Filter by subject ID
        #[arg(long)]
        subject_id: Option<String>,
    },

    /// Get a budget by ID
    Get {
        /// Budget ID (e.g. bdg_xxx)
        budget_id: String,
    },

    /// Update a budget
    Update {
        /// Budget ID (e.g. bdg_xxx)
        budget_id: String,

        /// New hard limit
        #[arg(long)]
        limit: Option<f64>,

        /// New soft limit
        #[arg(long)]
        soft_limit: Option<f64>,

        /// Clear the soft limit
        #[arg(long, conflicts_with = "soft_limit")]
        clear_soft_limit: bool,

        /// New status (active, paused, exhausted, disabled)
        #[arg(long)]
        status: Option<String>,
    },

    /// Delete (soft-delete) a budget
    Delete {
        /// Budget ID (e.g. bdg_xxx)
        budget_id: String,
    },

    /// Add credits to a budget
    TopUp {
        /// Budget ID (e.g. bdg_xxx)
        budget_id: String,

        /// Amount of credit to add
        #[arg(long)]
        amount: f64,

        /// Optional description for the ledger entry
        #[arg(long)]
        description: Option<String>,
    },

    /// Show ledger entries for a budget
    Ledger {
        /// Budget ID (e.g. bdg_xxx)
        budget_id: String,

        /// Max entries to return
        #[arg(long)]
        limit: Option<i64>,

        /// Number of entries to skip
        #[arg(long)]
        offset: Option<i64>,
    },

    /// Check a budget's current status
    Check {
        /// Budget ID (e.g. bdg_xxx)
        budget_id: String,
    },
}

pub async fn run(
    command: BudgetsCommand,
    client: &Everruns,
    output: OutputFormat,
    quiet: bool,
) -> Result<()> {
    match command {
        BudgetsCommand::Create {
            subject_type,
            subject_id,
            currency,
            limit,
            soft_limit,
        } => {
            let mut req = CreateBudgetRequest::new(&subject_type, &subject_id, &currency, limit);
            req.soft_limit = soft_limit;
            let budget = client
                .budgets()
                .create(req)
                .await
                .context("Failed to create budget")?;
            if output.is_text() {
                if quiet {
                    println!("{}", budget.id);
                } else {
                    println!("Created budget: {}", budget.id);
                    print_budget_text(&budget);
                }
            } else {
                output.print_value(&budget);
            }
            Ok(())
        }
        BudgetsCommand::List {
            subject_type,
            subject_id,
        } => {
            let budgets = client
                .budgets()
                .list(subject_type.as_deref(), subject_id.as_deref())
                .await
                .context("Failed to list budgets")?;
            if output.is_text() {
                if budgets.is_empty() {
                    println!("No budgets found");
                    return Ok(());
                }
                print_table_header(&[
                    ("ID", 28),
                    ("SUBJECT", 24),
                    ("LIMIT", 12),
                    ("BALANCE", 12),
                    ("STATUS", 10),
                ]);
                for b in &budgets {
                    let subject = format!("{}:{}", b.subject_type, b.subject_id);
                    let limit = format_amount(b.limit, &b.currency);
                    let balance = format_amount(b.balance, &b.currency);
                    let status = format!("{:?}", b.status).to_lowercase();
                    print_table_row(&[
                        (&b.id, 28),
                        (&subject, 24),
                        (&limit, 12),
                        (&balance, 12),
                        (&status, 10),
                    ]);
                }
            } else {
                output.print_value(&serde_json::json!({ "data": budgets }));
            }
            Ok(())
        }
        BudgetsCommand::Get { budget_id } => {
            let budget = client
                .budgets()
                .get(&budget_id)
                .await
                .map_err(|e| anyhow::anyhow!("Budget not found: {} ({})", budget_id, e))?;
            if output.is_text() {
                print_budget_text(&budget);
            } else {
                output.print_value(&budget);
            }
            Ok(())
        }
        BudgetsCommand::Update {
            budget_id,
            limit,
            soft_limit,
            clear_soft_limit,
            status,
        } => {
            let mut req = UpdateBudgetRequest::new();
            if let Some(l) = limit {
                req = req.limit(l);
            }
            if clear_soft_limit {
                req = req.soft_limit(None);
            } else if let Some(s) = soft_limit {
                req = req.soft_limit(Some(s));
            }
            if let Some(s) = status {
                req.status = Some(s);
            }
            let budget = client
                .budgets()
                .update(&budget_id, req)
                .await
                .context("Failed to update budget")?;
            if output.is_text() {
                if quiet {
                    println!("{}", budget.id);
                } else {
                    println!("Updated budget: {}", budget.id);
                    print_budget_text(&budget);
                }
            } else {
                output.print_value(&budget);
            }
            Ok(())
        }
        BudgetsCommand::Delete { budget_id } => {
            client
                .budgets()
                .delete(&budget_id)
                .await
                .map_err(|e| anyhow::anyhow!("Budget not found: {} ({})", budget_id, e))?;
            if output.is_text() {
                if !quiet {
                    println!("Deleted budget: {}", budget_id);
                }
            } else {
                output.print_value(&serde_json::json!({ "id": budget_id, "status": "deleted" }));
            }
            Ok(())
        }
        BudgetsCommand::TopUp {
            budget_id,
            amount,
            description,
        } => {
            let mut req = TopUpRequest::new(amount);
            if let Some(d) = description {
                req = req.description(d);
            }
            let budget = client
                .budgets()
                .top_up(&budget_id, req)
                .await
                .context("Failed to top up budget")?;
            if output.is_text() {
                if quiet {
                    println!("{}", format_amount(budget.balance, &budget.currency));
                } else {
                    println!("Topped up budget: {}", budget.id);
                    print_field("Balance", &format_amount(budget.balance, &budget.currency));
                }
            } else {
                output.print_value(&budget);
            }
            Ok(())
        }
        BudgetsCommand::Ledger {
            budget_id,
            limit,
            offset,
        } => {
            let entries = client
                .budgets()
                .ledger(&budget_id, limit, offset)
                .await
                .context("Failed to fetch budget ledger")?;
            if output.is_text() {
                if entries.is_empty() {
                    println!("No ledger entries");
                    return Ok(());
                }
                print_table_header(&[
                    ("CREATED", 22),
                    ("AMOUNT", 12),
                    ("SOURCE", 16),
                    ("DESCRIPTION", 30),
                ]);
                for e in &entries {
                    let amount = format!("{:.4}", e.amount);
                    let description = e.description.as_deref().unwrap_or("-");
                    print_table_row(&[
                        (&e.created_at, 22),
                        (&amount, 12),
                        (&e.meter_source, 16),
                        (description, 30),
                    ]);
                }
            } else {
                output.print_value(&serde_json::json!({ "data": entries }));
            }
            Ok(())
        }
        BudgetsCommand::Check { budget_id } => {
            let result = client
                .budgets()
                .check(&budget_id)
                .await
                .context("Failed to check budget")?;
            if output.is_text() {
                print_field("Action", &result.action);
                if let Some(msg) = &result.message {
                    print_field("Message", msg);
                }
                if let Some(balance) = result.balance {
                    let currency = result.currency.as_deref().unwrap_or("");
                    print_field("Balance", &format_amount(balance, currency));
                }
            } else {
                output.print_value(&result);
            }
            Ok(())
        }
    }
}

/// Print a budget in text mode.
fn print_budget_text(budget: &Budget) {
    print_field("ID", &budget.id);
    print_field(
        "Subject",
        &format!("{}:{}", budget.subject_type, budget.subject_id),
    );
    print_field("Currency", &budget.currency);
    print_field("Limit", &format_amount(budget.limit, &budget.currency));
    if let Some(soft) = budget.soft_limit {
        print_field("Soft limit", &format_amount(soft, &budget.currency));
    }
    print_field("Balance", &format_amount(budget.balance, &budget.currency));
    print_field("Status", &format!("{:?}", budget.status).to_lowercase());
    print_field("Created", &budget.created_at);
}

/// Format a monetary amount with its currency code (e.g. "10.0000 USD").
fn format_amount(amount: f64, currency: &str) -> String {
    if currency.is_empty() {
        format!("{:.4}", amount)
    } else {
        format!("{:.4} {}", amount, currency)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_amount_with_currency() {
        assert_eq!(format_amount(10.0, "USD"), "10.0000 USD");
        assert_eq!(format_amount(1.5, "EUR"), "1.5000 EUR");
    }

    #[test]
    fn test_format_amount_without_currency() {
        assert_eq!(format_amount(10.0, ""), "10.0000");
    }
}
