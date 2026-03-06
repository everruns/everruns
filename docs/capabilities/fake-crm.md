---
title: Fake CRM
description: Demo capability with simulated CRM and customer support tools
---

| | |
|---|---|
| **ID** | `fake_crm` |
| **Category** | Demo |
| **Features** | None |
| **Dependencies** | None |

Simulated CRM and customer support tools for testing and demonstrations. Manage customers, support tickets, and interaction history. State is persisted in the session filesystem.

## Tools

| Tool | Description |
|---|---|
| `crm_list_customers` | List customers with pagination |
| `crm_get_customer` | Get customer details by ID |
| `crm_create_customer` | Create a new customer |
| `crm_list_tickets` | List support tickets |
| `crm_create_ticket` | Create a support ticket |
| `crm_update_ticket` | Update ticket status |
| `crm_add_interaction` | Add a customer interaction note |
| `crm_search_customers` | Search customers by criteria |

## Use Cases

- **Support agent demos** — showcase agents handling customer inquiries and ticket management
- **Testing conversational workflows** — validate agents managing customer interactions
- **Prompt development** — develop customer service agent prompts with realistic data

## Example

```
User: A customer called about a billing issue, create a ticket

Agent:
  → crm_search_customers({ query: "billing issue" })
  ← []
  → crm_list_customers({ limit: 5 })
  ← [{ id: "cust-001", name: "Jane Smith", email: "jane@example.com" }, ...]

  "Which customer reported the billing issue?"

User: Jane Smith

Agent:
  → crm_create_ticket({
      customer_id: "cust-001",
      subject: "Billing discrepancy",
      priority: "high",
      description: "Customer reports incorrect charge on latest invoice"
    })
  ← { ticket_id: "TKT-042", status: "open" }
  → crm_add_interaction({
      customer_id: "cust-001",
      type: "phone",
      notes: "Customer called about billing discrepancy. Ticket TKT-042 created."
    })
```

## See Also

- [Fake Warehouse](/capabilities/fake-warehouse/) — simulated warehouse operations
- [Fake AWS](/capabilities/fake-aws/) — simulated cloud infrastructure
- [Capabilities Overview](/capabilities/)
