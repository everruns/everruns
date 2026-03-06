---
title: Fake Warehouse
description: Demo capability with simulated warehouse management tools
---

| | |
|---|---|
| **ID** | `fake_warehouse` |
| **Category** | Demo |
| **Risk** | Low |
| **Features** | None |
| **Dependencies** | None |

Simulated warehouse management tools for testing and demonstrations. Provides inventory tracking, shipment management, order processing, invoicing, and returns. State is persisted in the session filesystem.

## Tools

| Tool | Description |
|---|---|
| `warehouse_get_inventory` | Get current inventory levels |
| `warehouse_update_inventory` | Update item quantities |
| `warehouse_create_shipment` | Create a new shipment |
| `warehouse_list_shipments` | List all shipments |
| `warehouse_update_shipment_status` | Update shipment status |
| `warehouse_create_order` | Create a new order |
| `warehouse_list_orders` | List all orders |
| `warehouse_create_invoice` | Generate an invoice |
| `warehouse_process_return` | Process a return |
| `warehouse_inventory_report` | Generate inventory report |

## Use Cases

- **Agent demos** — showcase multi-tool agent workflows with realistic domain operations
- **Testing tool calling** — validate agent behavior with complex, stateful tool chains
- **Prompt engineering** — develop and test system prompts for warehouse management agents

## Example

```
User: Create an order for 50 widgets and ship it

Agent:
  → warehouse_create_order({ items: [{ sku: "WIDGET-001", quantity: 50 }], customer: "Acme Corp" })
  ← { order_id: "ORD-001", status: "created" }
  → warehouse_create_shipment({ order_id: "ORD-001", carrier: "FedEx" })
  ← { shipment_id: "SHP-001", status: "pending" }
  → warehouse_update_shipment_status({ shipment_id: "SHP-001", status: "shipped" })
  → warehouse_create_invoice({ order_id: "ORD-001" })
```

## See Also

- [Fake AWS](/capabilities/fake-aws/) — simulated cloud infrastructure
- [Fake CRM](/capabilities/fake-crm/) — simulated customer management
- [Capabilities Overview](/capabilities/)
