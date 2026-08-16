---
title: Fake CRM Capability for Support Agent Demos
description: Test support-agent workflows with a simulated CRM capability for mock customers, tickets, interactions, customer search, and session-persisted demo data.
sidebar:
  label: Fake CRM
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

## See Also

- [Fake Warehouse](/capabilities/fake-warehouse/), simulated warehouse operations
- [Fake AWS](/capabilities/fake-aws/), simulated cloud infrastructure
- [Capabilities Overview](/capabilities/)
