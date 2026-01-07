//! Fake Financial Tools Capability - demo tools for financial analysis
//!
//! This capability provides mock financial management tools that store state
//! in the session file system. Perfect for demos and testing financial workflows.
//!
//! Tools provided:
//! - `finance_list_transactions`: List financial transactions
//! - `finance_create_transaction`: Record a new transaction
//! - `finance_get_balance`: Get account balances
//! - `finance_list_budgets`: List budgets
//! - `finance_create_budget`: Create a new budget
//! - `finance_get_expense_report`: Generate expense report
//! - `finance_get_revenue_report`: Generate revenue report
//! - `finance_forecast_cash_flow`: Forecast cash flow

use super::{Capability, CapabilityId, CapabilityStatus};
use crate::tools::{Tool, ToolExecutionResult};
use crate::traits::ToolContext;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// Fake Financial Tools capability - mock financial management for demos
pub struct FakeFinancialCapability;

impl Capability for FakeFinancialCapability {
    fn id(&self) -> &str {
        CapabilityId::FAKE_FINANCIAL
    }

    fn name(&self) -> &str {
        "Fake Financial Tools"
    }

    fn description(&self) -> &str {
        "Demo capability: financial management tools (transactions, budgets, reports, forecasting). State stored in session filesystem."
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn icon(&self) -> Option<&str> {
        Some("dollar-sign")
    }

    fn category(&self) -> Option<&str> {
        Some("Demo Tools")
    }

    fn system_prompt_addition(&self) -> Option<&str> {
        Some(
            r#"You have access to financial management tools. All financial data is stored in /finance/ directory.

Available tools:
- `finance_list_transactions`: List financial transactions (filter by type/category)
- `finance_create_transaction`: Record a new transaction (income/expense)
- `finance_get_balance`: Get current account balances
- `finance_list_budgets`: List budgets by category
- `finance_create_budget`: Create or update a budget
- `finance_get_expense_report`: Generate expense report by period
- `finance_get_revenue_report`: Generate revenue report by period
- `finance_forecast_cash_flow`: Forecast cash flow for upcoming months

Data structure:
- /finance/transactions.json - Transaction records
- /finance/budgets.json - Budget definitions
- /finance/accounts.json - Account balances"#,
        )
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        vec![
            Box::new(FinanceListTransactionsTool),
            Box::new(FinanceCreateTransactionTool),
            Box::new(FinanceGetBalanceTool),
            Box::new(FinanceListBudgetsTool),
            Box::new(FinanceCreateBudgetTool),
            Box::new(FinanceGetExpenseReportTool),
            Box::new(FinanceGetRevenueReportTool),
            Box::new(FinanceForecastCashFlowTool),
        ]
    }
}

// Helper structs for financial data
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Transaction {
    id: String,
    date: String,
    transaction_type: String, // income, expense
    category: String,
    amount: f64,
    description: String,
    account: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Budget {
    category: String,
    monthly_limit: f64,
    current_spent: f64,
    period_start: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Account {
    name: String,
    balance: f64,
    currency: String,
}

// ============================================================================
// Tool: finance_list_transactions
// ============================================================================

pub struct FinanceListTransactionsTool;

#[async_trait]
impl Tool for FinanceListTransactionsTool {
    fn name(&self) -> &str {
        "finance_list_transactions"
    }

    fn description(&self) -> &str {
        "List financial transactions. Filter by type (income/expense) or category."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "transaction_type": {
                    "type": "string",
                    "enum": ["income", "expense"],
                    "description": "Optional: Filter by transaction type"
                },
                "category": {
                    "type": "string",
                    "description": "Optional: Filter by category"
                }
            },
            "additionalProperties": false
        })
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("finance_list_transactions requires context")
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let file_store = match &context.file_store {
            Some(store) => store,
            None => return ToolExecutionResult::tool_error("File system not available"),
        };

        let type_filter = arguments.get("transaction_type").and_then(|v| v.as_str());
        let category_filter = arguments.get("category").and_then(|v| v.as_str());

        // Read transactions
        let transactions: Vec<Transaction> = match file_store
            .read_file(context.session_id, "/finance/transactions.json")
            .await
        {
            Ok(Some(file)) => serde_json::from_str(file.content.as_deref().unwrap_or(""))
                .unwrap_or_else(|_| {
                    // Initialize with sample data
                    vec![
                        Transaction {
                            id: "TXN-001".to_string(),
                            date: "2025-01-05".to_string(),
                            transaction_type: "income".to_string(),
                            category: "sales".to_string(),
                            amount: 15000.0,
                            description: "Q1 Product Sales".to_string(),
                            account: "operating".to_string(),
                        },
                        Transaction {
                            id: "TXN-002".to_string(),
                            date: "2025-01-06".to_string(),
                            transaction_type: "expense".to_string(),
                            category: "payroll".to_string(),
                            amount: 8500.0,
                            description: "January Payroll".to_string(),
                            account: "operating".to_string(),
                        },
                    ]
                }),
            _ => {
                let initial = vec![Transaction {
                    id: "TXN-001".to_string(),
                    date: chrono::Utc::now().format("%Y-%m-%d").to_string(),
                    transaction_type: "income".to_string(),
                    category: "sales".to_string(),
                    amount: 15000.0,
                    description: "Initial Sale".to_string(),
                    account: "operating".to_string(),
                }];

                let content = serde_json::to_string_pretty(&initial).unwrap();
                let _ = file_store
                    .write_file(
                        context.session_id,
                        "/finance/transactions.json",
                        &content,
                        "text",
                    )
                    .await;

                initial
            }
        };

        // Apply filters
        let mut filtered = transactions;
        if let Some(tx_type) = type_filter {
            filtered.retain(|t| t.transaction_type == tx_type);
        }
        if let Some(category) = category_filter {
            filtered.retain(|t| t.category == category);
        }

        let total: f64 = filtered.iter().map(|t| t.amount).sum();

        ToolExecutionResult::success(json!({
            "transactions": filtered,
            "total_count": filtered.len(),
            "total_amount": total
        }))
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// ============================================================================
// Tool: finance_create_transaction
// ============================================================================

pub struct FinanceCreateTransactionTool;

#[async_trait]
impl Tool for FinanceCreateTransactionTool {
    fn name(&self) -> &str {
        "finance_create_transaction"
    }

    fn description(&self) -> &str {
        "Record a new financial transaction (income or expense)."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "transaction_type": {
                    "type": "string",
                    "enum": ["income", "expense"]
                },
                "category": {"type": "string"},
                "amount": {"type": "number", "minimum": 0},
                "description": {"type": "string"},
                "account": {"type": "string", "default": "operating"}
            },
            "required": ["transaction_type", "category", "amount", "description"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("finance_create_transaction requires context")
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let file_store = match &context.file_store {
            Some(store) => store,
            None => return ToolExecutionResult::tool_error("File system not available"),
        };

        let transaction_type = match arguments.get("transaction_type").and_then(|v| v.as_str()) {
            Some(t) => t,
            None => {
                return ToolExecutionResult::tool_error(
                    "Missing required parameter: transaction_type",
                )
            }
        };

        let category = match arguments.get("category").and_then(|v| v.as_str()) {
            Some(c) => c,
            None => return ToolExecutionResult::tool_error("Missing required parameter: category"),
        };

        let amount = match arguments.get("amount").and_then(|v| v.as_f64()) {
            Some(a) => a,
            None => return ToolExecutionResult::tool_error("Missing required parameter: amount"),
        };

        let description = match arguments.get("description").and_then(|v| v.as_str()) {
            Some(d) => d,
            None => {
                return ToolExecutionResult::tool_error("Missing required parameter: description")
            }
        };

        let account = arguments
            .get("account")
            .and_then(|v| v.as_str())
            .unwrap_or("operating");

        // Read current transactions
        let mut transactions: Vec<Transaction> = match file_store
            .read_file(context.session_id, "/finance/transactions.json")
            .await
        {
            Ok(Some(file)) => {
                serde_json::from_str(file.content.as_deref().unwrap_or("")).unwrap_or_default()
            }
            _ => vec![],
        };

        // Create new transaction
        let transaction_id = format!("TXN-{:03}", transactions.len() + 1);
        let date = chrono::Utc::now().format("%Y-%m-%d").to_string();

        let transaction = Transaction {
            id: transaction_id.clone(),
            date: date.clone(),
            transaction_type: transaction_type.to_string(),
            category: category.to_string(),
            amount,
            description: description.to_string(),
            account: account.to_string(),
        };

        transactions.push(transaction.clone());

        // Save transactions
        let content = serde_json::to_string_pretty(&transactions).unwrap();
        match file_store
            .write_file(
                context.session_id,
                "/finance/transactions.json",
                &content,
                "text",
            )
            .await
        {
            Ok(_) => ToolExecutionResult::success(json!({
                "transaction_id": transaction_id,
                "amount": amount,
                "type": transaction_type,
                "date": date
            })),
            Err(e) => ToolExecutionResult::internal_error(e),
        }
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// ============================================================================
// Tool: finance_get_balance
// ============================================================================

pub struct FinanceGetBalanceTool;

#[async_trait]
impl Tool for FinanceGetBalanceTool {
    fn name(&self) -> &str {
        "finance_get_balance"
    }

    fn description(&self) -> &str {
        "Get current account balances."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "account": {
                    "type": "string",
                    "description": "Optional: Get balance for specific account"
                }
            },
            "additionalProperties": false
        })
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("finance_get_balance requires context")
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        _context: &ToolContext,
    ) -> ToolExecutionResult {
        let account_filter = arguments.get("account").and_then(|v| v.as_str());

        let accounts = vec![
            Account {
                name: "operating".to_string(),
                balance: 125000.0,
                currency: "USD".to_string(),
            },
            Account {
                name: "savings".to_string(),
                balance: 50000.0,
                currency: "USD".to_string(),
            },
            Account {
                name: "payroll".to_string(),
                balance: 75000.0,
                currency: "USD".to_string(),
            },
        ];

        let filtered: Vec<_> = if let Some(account) = account_filter {
            accounts.into_iter().filter(|a| a.name == account).collect()
        } else {
            accounts
        };

        let total_balance: f64 = filtered.iter().map(|a| a.balance).sum();

        ToolExecutionResult::success(json!({
            "accounts": filtered,
            "total_balance": total_balance,
            "currency": "USD"
        }))
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// ============================================================================
// Remaining tools (simplified implementations)
// ============================================================================

pub struct FinanceListBudgetsTool;

#[async_trait]
impl Tool for FinanceListBudgetsTool {
    fn name(&self) -> &str {
        "finance_list_budgets"
    }

    fn description(&self) -> &str {
        "List all budgets by category."
    }

    fn parameters_schema(&self) -> Value {
        json!({"type": "object", "additionalProperties": false})
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("Requires context")
    }

    async fn execute_with_context(
        &self,
        _arguments: Value,
        _context: &ToolContext,
    ) -> ToolExecutionResult {
        let budgets = vec![
            Budget {
                category: "payroll".to_string(),
                monthly_limit: 50000.0,
                current_spent: 38500.0,
                period_start: "2025-01-01".to_string(),
            },
            Budget {
                category: "marketing".to_string(),
                monthly_limit: 15000.0,
                current_spent: 8200.0,
                period_start: "2025-01-01".to_string(),
            },
        ];

        ToolExecutionResult::success(json!({"budgets": budgets, "total_count": budgets.len()}))
    }

    fn requires_context(&self) -> bool {
        true
    }
}

pub struct FinanceCreateBudgetTool;

#[async_trait]
impl Tool for FinanceCreateBudgetTool {
    fn name(&self) -> &str {
        "finance_create_budget"
    }

    fn description(&self) -> &str {
        "Create or update a budget for a category."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "category": {"type": "string"},
                "monthly_limit": {"type": "number", "minimum": 0}
            },
            "required": ["category", "monthly_limit"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("Requires context")
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        _context: &ToolContext,
    ) -> ToolExecutionResult {
        let category = arguments
            .get("category")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let monthly_limit = arguments
            .get("monthly_limit")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);

        ToolExecutionResult::success(json!({
            "category": category,
            "monthly_limit": monthly_limit,
            "status": "created"
        }))
    }

    fn requires_context(&self) -> bool {
        true
    }
}

pub struct FinanceGetExpenseReportTool;

#[async_trait]
impl Tool for FinanceGetExpenseReportTool {
    fn name(&self) -> &str {
        "finance_get_expense_report"
    }

    fn description(&self) -> &str {
        "Generate expense report for a time period, broken down by category."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "period": {
                    "type": "string",
                    "enum": ["current_month", "last_month", "quarter", "year"],
                    "default": "current_month"
                }
            },
            "additionalProperties": false
        })
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("Requires context")
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        _context: &ToolContext,
    ) -> ToolExecutionResult {
        let period = arguments
            .get("period")
            .and_then(|v| v.as_str())
            .unwrap_or("current_month");

        let expense_breakdown = json!([
            {"category": "payroll", "amount": 85000.0, "percentage": 60.0},
            {"category": "marketing", "amount": 25000.0, "percentage": 18.0},
            {"category": "operations", "amount": 20000.0, "percentage": 14.0},
            {"category": "other", "amount": 11500.0, "percentage": 8.0}
        ]);

        ToolExecutionResult::success(json!({
            "period": period,
            "total_expenses": 141500.0,
            "breakdown": expense_breakdown,
            "top_category": "payroll"
        }))
    }

    fn requires_context(&self) -> bool {
        true
    }
}

pub struct FinanceGetRevenueReportTool;

#[async_trait]
impl Tool for FinanceGetRevenueReportTool {
    fn name(&self) -> &str {
        "finance_get_revenue_report"
    }

    fn description(&self) -> &str {
        "Generate revenue report for a time period."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "period": {
                    "type": "string",
                    "enum": ["current_month", "last_month", "quarter", "year"],
                    "default": "current_month"
                }
            },
            "additionalProperties": false
        })
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("Requires context")
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        _context: &ToolContext,
    ) -> ToolExecutionResult {
        let period = arguments
            .get("period")
            .and_then(|v| v.as_str())
            .unwrap_or("current_month");

        let revenue_breakdown = json!([
            {"source": "product_sales", "amount": 150000.0, "percentage": 75.0},
            {"source": "subscriptions", "amount": 35000.0, "percentage": 17.5},
            {"source": "services", "amount": 15000.0, "percentage": 7.5}
        ]);

        ToolExecutionResult::success(json!({
            "period": period,
            "total_revenue": 200000.0,
            "breakdown": revenue_breakdown,
            "growth_vs_last_period": 12.5
        }))
    }

    fn requires_context(&self) -> bool {
        true
    }
}

pub struct FinanceForecastCashFlowTool;

#[async_trait]
impl Tool for FinanceForecastCashFlowTool {
    fn name(&self) -> &str {
        "finance_forecast_cash_flow"
    }

    fn description(&self) -> &str {
        "Forecast cash flow for the next 3-6 months based on historical data."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "months": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 12,
                    "default": 3,
                    "description": "Number of months to forecast"
                }
            },
            "additionalProperties": false
        })
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("Requires context")
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        _context: &ToolContext,
    ) -> ToolExecutionResult {
        let months = arguments
            .get("months")
            .and_then(|v| v.as_i64())
            .unwrap_or(3) as i32;

        let forecast: Vec<Value> = (1..=months)
            .map(|m| {
                json!({
                    "month": m,
                    "projected_revenue": 200000.0 + (m as f64 * 5000.0),
                    "projected_expenses": 140000.0 + (m as f64 * 2000.0),
                    "net_cash_flow": 60000.0 + (m as f64 * 3000.0),
                    "ending_balance": 125000.0 + (m as f64 * 60000.0)
                })
            })
            .collect();

        ToolExecutionResult::success(json!({
            "forecast_months": months,
            "forecast": forecast,
            "assumptions": "Based on 10% monthly growth"
        }))
    }

    fn requires_context(&self) -> bool {
        true
    }
}
