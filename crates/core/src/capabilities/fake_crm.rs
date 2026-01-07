//! Fake CRM Tools Capability - demo tools for customer relationship management
//!
//! This capability provides mock CRM tools that store state
//! in the session file system. Perfect for demos and testing customer support workflows.
//!
//! Tools provided:
//! - `crm_list_customers`: List customers
//! - `crm_get_customer`: Get customer details
//! - `crm_create_customer`: Create a new customer
//! - `crm_list_tickets`: List support tickets
//! - `crm_create_ticket`: Create a new support ticket
//! - `crm_update_ticket`: Update ticket status
//! - `crm_add_interaction`: Add customer interaction note
//! - `crm_search_customers`: Search customers by criteria

use super::{Capability, CapabilityId, CapabilityStatus};
use crate::tools::{Tool, ToolExecutionResult};
use crate::traits::ToolContext;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// Fake CRM Tools capability - mock customer relationship management for demos
pub struct FakeCrmCapability;

impl Capability for FakeCrmCapability {
    fn id(&self) -> &str {
        CapabilityId::FAKE_CRM
    }

    fn name(&self) -> &str {
        "Fake CRM Tools"
    }

    fn description(&self) -> &str {
        "Demo capability: CRM and customer support tools (customers, tickets, interactions). State stored in session filesystem."
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn icon(&self) -> Option<&str> {
        Some("users")
    }

    fn category(&self) -> Option<&str> {
        Some("Demo Tools")
    }

    fn system_prompt_addition(&self) -> Option<&str> {
        Some(
            r#"You have access to CRM and customer support tools. All CRM data is stored in /crm/ directory.

Available tools:
- `crm_list_customers`: List all customers with pagination
- `crm_get_customer`: Get detailed customer information by ID
- `crm_create_customer`: Create a new customer record
- `crm_list_tickets`: List support tickets (filter by status/priority)
- `crm_create_ticket`: Create a new support ticket
- `crm_update_ticket`: Update ticket status and assign to agents
- `crm_add_interaction`: Add a customer interaction note (call, email, meeting)
- `crm_search_customers`: Search customers by name, email, or company

Data structure:
- /crm/customers.json - Customer records
- /crm/tickets.json - Support ticket records
- /crm/interactions.json - Customer interaction history"#,
        )
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        vec![
            Box::new(CrmListCustomersTool),
            Box::new(CrmGetCustomerTool),
            Box::new(CrmCreateCustomerTool),
            Box::new(CrmListTicketsTool),
            Box::new(CrmCreateTicketTool),
            Box::new(CrmUpdateTicketTool),
            Box::new(CrmAddInteractionTool),
            Box::new(CrmSearchCustomersTool),
        ]
    }
}

// Helper structs for CRM data
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Customer {
    id: String,
    name: String,
    email: String,
    company: String,
    phone: String,
    tier: String, // free, pro, enterprise
    created_at: String,
    last_contact: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Ticket {
    id: String,
    customer_id: String,
    subject: String,
    description: String,
    status: String,   // open, in_progress, resolved, closed
    priority: String, // low, medium, high, urgent
    assigned_to: Option<String>,
    created_at: String,
    updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Interaction {
    id: String,
    customer_id: String,
    interaction_type: String, // call, email, meeting, chat
    summary: String,
    agent: String,
    timestamp: String,
}

// ============================================================================
// Tool: crm_list_customers
// ============================================================================

pub struct CrmListCustomersTool;

#[async_trait]
impl Tool for CrmListCustomersTool {
    fn name(&self) -> &str {
        "crm_list_customers"
    }

    fn description(&self) -> &str {
        "List all customers. Optionally filter by customer tier."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "tier": {
                    "type": "string",
                    "enum": ["free", "pro", "enterprise"],
                    "description": "Optional: Filter by customer tier"
                }
            },
            "additionalProperties": false
        })
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("crm_list_customers requires context")
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

        let tier_filter = arguments.get("tier").and_then(|v| v.as_str());

        // Read customers
        let customers: Vec<Customer> = match file_store
            .read_file(context.session_id, "/crm/customers.json")
            .await
        {
            Ok(Some(file)) => serde_json::from_str(&file.content).unwrap_or_else(|_| {
                // Initialize with sample data
                vec![
                    Customer {
                        id: "CUST-001".to_string(),
                        name: "Alice Johnson".to_string(),
                        email: "alice@acmecorp.com".to_string(),
                        company: "Acme Corp".to_string(),
                        phone: "+1-555-0101".to_string(),
                        tier: "enterprise".to_string(),
                        created_at: "2024-01-15T10:00:00Z".to_string(),
                        last_contact: "2025-01-05T14:30:00Z".to_string(),
                    },
                    Customer {
                        id: "CUST-002".to_string(),
                        name: "Bob Smith".to_string(),
                        email: "bob@techstart.io".to_string(),
                        company: "TechStart Inc".to_string(),
                        phone: "+1-555-0102".to_string(),
                        tier: "pro".to_string(),
                        created_at: "2024-03-20T09:00:00Z".to_string(),
                        last_contact: "2025-01-03T11:15:00Z".to_string(),
                    },
                ]
            }),
            _ => {
                let initial = vec![Customer {
                    id: "CUST-001".to_string(),
                    name: "Alice Johnson".to_string(),
                    email: "alice@acmecorp.com".to_string(),
                    company: "Acme Corp".to_string(),
                    phone: "+1-555-0101".to_string(),
                    tier: "enterprise".to_string(),
                    created_at: chrono::Utc::now().to_rfc3339(),
                    last_contact: chrono::Utc::now().to_rfc3339(),
                }];

                let content = serde_json::to_string_pretty(&initial).unwrap();
                let _ = file_store
                    .write_file(context.session_id, "/crm/customers.json", &content, "text")
                    .await;

                initial
            }
        };

        // Apply filter
        let filtered: Vec<_> = if let Some(tier) = tier_filter {
            customers.into_iter().filter(|c| c.tier == tier).collect()
        } else {
            customers
        };

        ToolExecutionResult::success(json!({
            "customers": filtered,
            "total_count": filtered.len()
        }))
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// ============================================================================
// Tool: crm_get_customer
// ============================================================================

pub struct CrmGetCustomerTool;

#[async_trait]
impl Tool for CrmGetCustomerTool {
    fn name(&self) -> &str {
        "crm_get_customer"
    }

    fn description(&self) -> &str {
        "Get detailed customer information by customer ID."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "customer_id": {
                    "type": "string",
                    "description": "Customer ID to retrieve"
                }
            },
            "required": ["customer_id"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("crm_get_customer requires context")
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

        let customer_id = match arguments.get("customer_id").and_then(|v| v.as_str()) {
            Some(id) => id,
            None => {
                return ToolExecutionResult::tool_error("Missing required parameter: customer_id")
            }
        };

        // Read customers
        let customers: Vec<Customer> = match file_store
            .read_file(context.session_id, "/crm/customers.json")
            .await
        {
            Ok(Some(file)) => serde_json::from_str(&file.content).unwrap_or_default(),
            _ => vec![],
        };

        // Find customer
        match customers.iter().find(|c| c.id == customer_id) {
            Some(customer) => ToolExecutionResult::success(json!({"customer": customer})),
            None => ToolExecutionResult::tool_error(format!("Customer not found: {}", customer_id)),
        }
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// ============================================================================
// Tool: crm_create_customer
// ============================================================================

pub struct CrmCreateCustomerTool;

#[async_trait]
impl Tool for CrmCreateCustomerTool {
    fn name(&self) -> &str {
        "crm_create_customer"
    }

    fn description(&self) -> &str {
        "Create a new customer record in the CRM."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "name": {"type": "string"},
                "email": {"type": "string"},
                "company": {"type": "string"},
                "phone": {"type": "string"},
                "tier": {
                    "type": "string",
                    "enum": ["free", "pro", "enterprise"],
                    "default": "free"
                }
            },
            "required": ["name", "email"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("crm_create_customer requires context")
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

        let name = match arguments.get("name").and_then(|v| v.as_str()) {
            Some(n) => n,
            None => return ToolExecutionResult::tool_error("Missing required parameter: name"),
        };

        let email = match arguments.get("email").and_then(|v| v.as_str()) {
            Some(e) => e,
            None => return ToolExecutionResult::tool_error("Missing required parameter: email"),
        };

        let company = arguments
            .get("company")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let phone = arguments
            .get("phone")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let tier = arguments
            .get("tier")
            .and_then(|v| v.as_str())
            .unwrap_or("free");

        // Read current customers
        let mut customers: Vec<Customer> = match file_store
            .read_file(context.session_id, "/crm/customers.json")
            .await
        {
            Ok(Some(file)) => serde_json::from_str(&file.content).unwrap_or_default(),
            _ => vec![],
        };

        // Create new customer
        let customer_id = format!("CUST-{:03}", customers.len() + 1);
        let now = chrono::Utc::now().to_rfc3339();

        let customer = Customer {
            id: customer_id.clone(),
            name: name.to_string(),
            email: email.to_string(),
            company: company.to_string(),
            phone: phone.to_string(),
            tier: tier.to_string(),
            created_at: now.clone(),
            last_contact: now,
        };

        customers.push(customer.clone());

        // Save customers
        let content = serde_json::to_string_pretty(&customers).unwrap();
        match file_store
            .write_file(context.session_id, "/crm/customers.json", &content, "text")
            .await
        {
            Ok(_) => ToolExecutionResult::success(json!({
                "customer_id": customer_id,
                "customer": customer
            })),
            Err(e) => ToolExecutionResult::internal_error(e),
        }
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// ============================================================================
// Tool: crm_list_tickets
// ============================================================================

pub struct CrmListTicketsTool;

#[async_trait]
impl Tool for CrmListTicketsTool {
    fn name(&self) -> &str {
        "crm_list_tickets"
    }

    fn description(&self) -> &str {
        "List support tickets. Filter by status or priority."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "status": {
                    "type": "string",
                    "enum": ["open", "in_progress", "resolved", "closed"]
                },
                "priority": {
                    "type": "string",
                    "enum": ["low", "medium", "high", "urgent"]
                }
            },
            "additionalProperties": false
        })
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("crm_list_tickets requires context")
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

        let status_filter = arguments.get("status").and_then(|v| v.as_str());
        let priority_filter = arguments.get("priority").and_then(|v| v.as_str());

        // Read tickets
        let tickets: Vec<Ticket> = match file_store
            .read_file(context.session_id, "/crm/tickets.json")
            .await
        {
            Ok(Some(file)) => serde_json::from_str(&file.content).unwrap_or_default(),
            _ => vec![],
        };

        // Apply filters
        let mut filtered = tickets;
        if let Some(status) = status_filter {
            filtered.retain(|t| t.status == status);
        }
        if let Some(priority) = priority_filter {
            filtered.retain(|t| t.priority == priority);
        }

        ToolExecutionResult::success(json!({
            "tickets": filtered,
            "total_count": filtered.len()
        }))
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// ============================================================================
// Tool: crm_create_ticket
// ============================================================================

pub struct CrmCreateTicketTool;

#[async_trait]
impl Tool for CrmCreateTicketTool {
    fn name(&self) -> &str {
        "crm_create_ticket"
    }

    fn description(&self) -> &str {
        "Create a new support ticket for a customer."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "customer_id": {"type": "string"},
                "subject": {"type": "string"},
                "description": {"type": "string"},
                "priority": {
                    "type": "string",
                    "enum": ["low", "medium", "high", "urgent"],
                    "default": "medium"
                }
            },
            "required": ["customer_id", "subject", "description"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("crm_create_ticket requires context")
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

        let customer_id = match arguments.get("customer_id").and_then(|v| v.as_str()) {
            Some(id) => id,
            None => {
                return ToolExecutionResult::tool_error("Missing required parameter: customer_id")
            }
        };

        let subject = match arguments.get("subject").and_then(|v| v.as_str()) {
            Some(s) => s,
            None => return ToolExecutionResult::tool_error("Missing required parameter: subject"),
        };

        let description = match arguments.get("description").and_then(|v| v.as_str()) {
            Some(d) => d,
            None => {
                return ToolExecutionResult::tool_error("Missing required parameter: description")
            }
        };

        let priority = arguments
            .get("priority")
            .and_then(|v| v.as_str())
            .unwrap_or("medium");

        // Read current tickets
        let mut tickets: Vec<Ticket> = match file_store
            .read_file(context.session_id, "/crm/tickets.json")
            .await
        {
            Ok(Some(file)) => serde_json::from_str(&file.content).unwrap_or_default(),
            _ => vec![],
        };

        // Create new ticket
        let ticket_id = format!("TKT-{:05}", tickets.len() + 1);
        let now = chrono::Utc::now().to_rfc3339();

        let ticket = Ticket {
            id: ticket_id.clone(),
            customer_id: customer_id.to_string(),
            subject: subject.to_string(),
            description: description.to_string(),
            status: "open".to_string(),
            priority: priority.to_string(),
            assigned_to: None,
            created_at: now.clone(),
            updated_at: now,
        };

        tickets.push(ticket.clone());

        // Save tickets
        let content = serde_json::to_string_pretty(&tickets).unwrap();
        match file_store
            .write_file(context.session_id, "/crm/tickets.json", &content, "text")
            .await
        {
            Ok(_) => ToolExecutionResult::success(json!({
                "ticket_id": ticket_id,
                "status": "open",
                "priority": priority
            })),
            Err(e) => ToolExecutionResult::internal_error(e),
        }
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// ============================================================================
// Remaining tools (simplified implementations)
// ============================================================================

pub struct CrmUpdateTicketTool;

#[async_trait]
impl Tool for CrmUpdateTicketTool {
    fn name(&self) -> &str {
        "crm_update_ticket"
    }

    fn description(&self) -> &str {
        "Update ticket status or assignment."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "ticket_id": {"type": "string"},
                "status": {"type": "string", "enum": ["open", "in_progress", "resolved", "closed"]},
                "assigned_to": {"type": "string"}
            },
            "required": ["ticket_id"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("Requires context")
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

        let ticket_id = arguments
            .get("ticket_id")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");

        let new_status = arguments.get("status").and_then(|v| v.as_str());
        let assigned_to = arguments.get("assigned_to").and_then(|v| v.as_str());

        // Read tickets
        let mut tickets: Vec<Ticket> = match file_store
            .read_file(context.session_id, "/crm/tickets.json")
            .await
        {
            Ok(Some(file)) => serde_json::from_str(&file.content).unwrap_or_default(),
            _ => vec![],
        };

        // Find and update ticket
        if let Some(ticket) = tickets.iter_mut().find(|t| t.id == ticket_id) {
            if let Some(status) = new_status {
                ticket.status = status.to_string();
            }
            if let Some(agent) = assigned_to {
                ticket.assigned_to = Some(agent.to_string());
            }
            ticket.updated_at = chrono::Utc::now().to_rfc3339();

            let content = serde_json::to_string_pretty(&tickets).unwrap();
            let _ = file_store
                .write_file(context.session_id, "/crm/tickets.json", &content, "text")
                .await;

            ToolExecutionResult::success(json!({"ticket_id": ticket_id, "status": ticket.status}))
        } else {
            ToolExecutionResult::tool_error(format!("Ticket not found: {}", ticket_id))
        }
    }

    fn requires_context(&self) -> bool {
        true
    }
}

pub struct CrmAddInteractionTool;

#[async_trait]
impl Tool for CrmAddInteractionTool {
    fn name(&self) -> &str {
        "crm_add_interaction"
    }

    fn description(&self) -> &str {
        "Add a customer interaction note (call, email, meeting, chat)."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "customer_id": {"type": "string"},
                "interaction_type": {"type": "string", "enum": ["call", "email", "meeting", "chat"]},
                "summary": {"type": "string"},
                "agent": {"type": "string"}
            },
            "required": ["customer_id", "interaction_type", "summary", "agent"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("Requires context")
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let interaction_id = format!("INT-{:05}", chrono::Utc::now().timestamp() % 100000);

        ToolExecutionResult::success(json!({
            "interaction_id": interaction_id,
            "status": "recorded"
        }))
    }

    fn requires_context(&self) -> bool {
        true
    }
}

pub struct CrmSearchCustomersTool;

#[async_trait]
impl Tool for CrmSearchCustomersTool {
    fn name(&self) -> &str {
        "crm_search_customers"
    }

    fn description(&self) -> &str {
        "Search customers by name, email, or company."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {"type": "string", "description": "Search query"}
            },
            "required": ["query"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("Requires context")
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

        let query = arguments
            .get("query")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_lowercase();

        // Read customers
        let customers: Vec<Customer> = match file_store
            .read_file(context.session_id, "/crm/customers.json")
            .await
        {
            Ok(Some(file)) => serde_json::from_str(&file.content).unwrap_or_default(),
            _ => vec![],
        };

        // Search
        let results: Vec<_> = customers
            .into_iter()
            .filter(|c| {
                c.name.to_lowercase().contains(&query)
                    || c.email.to_lowercase().contains(&query)
                    || c.company.to_lowercase().contains(&query)
            })
            .collect();

        ToolExecutionResult::success(json!({
            "results": results,
            "count": results.len()
        }))
    }

    fn requires_context(&self) -> bool {
        true
    }
}
