---
title: Fake Warehouse
description: Demo capability with simulated warehouse management tools. Use this built-in demo to test agent workflows with mock inventory, orders, and shipping operations.
---

| | |
|---|---|
| **ID** | `fake_warehouse` |
| **Category** | Demo |
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

## See Also

- [Fake AWS](/capabilities/fake-aws/), simulated cloud infrastructure
- [Fake CRM](/capabilities/fake-crm/), simulated customer management
- [Capabilities Overview](/capabilities/)
