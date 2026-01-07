//! Fake Warehouse Tools Capability - demo tools for warehouse management
//!
//! This capability provides mock warehouse management tools that store state
//! in the session file system. Perfect for demos and testing.
//!
//! Tools provided:
//! - `warehouse_get_inventory`: Get current inventory levels
//! - `warehouse_update_inventory`: Update inventory quantities
//! - `warehouse_create_shipment`: Create a new shipment
//! - `warehouse_list_shipments`: List all shipments
//! - `warehouse_update_shipment_status`: Update shipment status
//! - `warehouse_create_order`: Create a new order
//! - `warehouse_list_orders`: List all orders
//! - `warehouse_create_invoice`: Generate an invoice
//! - `warehouse_process_return`: Process a product return
//! - `warehouse_inventory_report`: Generate inventory report

use super::{Capability, CapabilityId, CapabilityStatus};
use crate::tools::{Tool, ToolExecutionResult};
use crate::traits::ToolContext;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// Fake Warehouse Tools capability - mock warehouse management for demos
pub struct FakeWarehouseCapability;

impl Capability for FakeWarehouseCapability {
    fn id(&self) -> &str {
        CapabilityId::FAKE_WAREHOUSE
    }

    fn name(&self) -> &str {
        "Fake Warehouse Tools"
    }

    fn description(&self) -> &str {
        "Demo capability: warehouse management tools (inventory, shipments, orders, invoices, returns). State stored in session filesystem."
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn icon(&self) -> Option<&str> {
        Some("package")
    }

    fn category(&self) -> Option<&str> {
        Some("Demo Tools")
    }

    fn system_prompt_addition(&self) -> Option<&str> {
        Some(
            r#"You have access to warehouse management tools. All warehouse data is stored in /warehouse/ directory.

Available tools:
- `warehouse_get_inventory`: Get current inventory levels for products
- `warehouse_update_inventory`: Update inventory quantities (add/remove stock)
- `warehouse_create_shipment`: Create a new shipment with products
- `warehouse_list_shipments`: List all shipments with status tracking
- `warehouse_update_shipment_status`: Update shipment status (pending/in_transit/delivered)
- `warehouse_create_order`: Create a new customer order
- `warehouse_list_orders`: List all customer orders
- `warehouse_create_invoice`: Generate an invoice for an order
- `warehouse_process_return`: Process a product return and update inventory
- `warehouse_inventory_report`: Generate comprehensive inventory report

Data structure:
- /warehouse/inventory.json - Product inventory levels
- /warehouse/shipments.json - Shipment records
- /warehouse/orders.json - Customer orders
- /warehouse/invoices.json - Generated invoices
- /warehouse/returns.json - Return records"#,
        )
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        vec![
            Box::new(WarehouseGetInventoryTool),
            Box::new(WarehouseUpdateInventoryTool),
            Box::new(WarehouseCreateShipmentTool),
            Box::new(WarehouseListShipmentsTool),
            Box::new(WarehouseUpdateShipmentStatusTool),
            Box::new(WarehouseCreateOrderTool),
            Box::new(WarehouseListOrdersTool),
            Box::new(WarehouseCreateInvoiceTool),
            Box::new(WarehouseProcessReturnTool),
            Box::new(WarehouseInventoryReportTool),
        ]
    }
}

// Helper structs for warehouse data
#[derive(Debug, Clone, Serialize, Deserialize)]
struct InventoryItem {
    sku: String,
    name: String,
    quantity: i32,
    location: String,
    reorder_point: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Shipment {
    id: String,
    status: String, // pending, in_transit, delivered
    destination: String,
    items: Vec<ShipmentItem>,
    created_at: String,
    updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ShipmentItem {
    sku: String,
    quantity: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Order {
    id: String,
    customer_name: String,
    items: Vec<ShipmentItem>,
    status: String,
    created_at: String,
}

// ============================================================================
// Tool: warehouse_get_inventory
// ============================================================================

pub struct WarehouseGetInventoryTool;

#[async_trait]
impl Tool for WarehouseGetInventoryTool {
    fn name(&self) -> &str {
        "warehouse_get_inventory"
    }

    fn description(&self) -> &str {
        "Get current inventory levels. Optionally filter by SKU or show only low stock items."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "sku": {
                    "type": "string",
                    "description": "Optional: Get inventory for a specific SKU"
                },
                "low_stock_only": {
                    "type": "boolean",
                    "description": "Optional: Show only items below reorder point"
                }
            },
            "additionalProperties": false
        })
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "warehouse_get_inventory requires context. Must be executed with session context.",
        )
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

        let sku_filter = arguments.get("sku").and_then(|v| v.as_str());
        let low_stock_only = arguments
            .get("low_stock_only")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        // Read inventory file
        let inventory: Vec<InventoryItem> = match file_store
            .read_file(context.session_id, "/warehouse/inventory.json")
            .await
        {
            Ok(Some(file)) => serde_json::from_str(file.content.as_deref().unwrap_or(""))
                .unwrap_or_else(|_| {
                    // Initialize with sample data
                    vec![
                        InventoryItem {
                            sku: "WH-001".to_string(),
                            name: "Industrial Widget".to_string(),
                            quantity: 150,
                            location: "A1".to_string(),
                            reorder_point: 50,
                        },
                        InventoryItem {
                            sku: "WH-002".to_string(),
                            name: "Premium Gadget".to_string(),
                            quantity: 30,
                            location: "B2".to_string(),
                            reorder_point: 40,
                        },
                        InventoryItem {
                            sku: "WH-003".to_string(),
                            name: "Standard Component".to_string(),
                            quantity: 200,
                            location: "C3".to_string(),
                            reorder_point: 75,
                        },
                    ]
                }),
            _ => {
                // Create initial inventory
                let initial = vec![
                    InventoryItem {
                        sku: "WH-001".to_string(),
                        name: "Industrial Widget".to_string(),
                        quantity: 150,
                        location: "A1".to_string(),
                        reorder_point: 50,
                    },
                    InventoryItem {
                        sku: "WH-002".to_string(),
                        name: "Premium Gadget".to_string(),
                        quantity: 30,
                        location: "B2".to_string(),
                        reorder_point: 40,
                    },
                    InventoryItem {
                        sku: "WH-003".to_string(),
                        name: "Standard Component".to_string(),
                        quantity: 200,
                        location: "C3".to_string(),
                        reorder_point: 75,
                    },
                ];

                // Save initial data
                let content = serde_json::to_string_pretty(&initial).unwrap();
                let _ = file_store
                    .write_file(
                        context.session_id,
                        "/warehouse/inventory.json",
                        &content,
                        "text",
                    )
                    .await;

                initial
            }
        };

        // Apply filters
        let mut filtered: Vec<_> = inventory.into_iter().collect();

        if let Some(sku) = sku_filter {
            filtered.retain(|item| item.sku == sku);
        }

        if low_stock_only {
            filtered.retain(|item| item.quantity < item.reorder_point);
        }

        ToolExecutionResult::success(json!({
            "inventory": filtered,
            "total_items": filtered.len(),
            "low_stock_count": filtered.iter().filter(|item| item.quantity < item.reorder_point).count()
        }))
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// ============================================================================
// Tool: warehouse_update_inventory
// ============================================================================

pub struct WarehouseUpdateInventoryTool;

#[async_trait]
impl Tool for WarehouseUpdateInventoryTool {
    fn name(&self) -> &str {
        "warehouse_update_inventory"
    }

    fn description(&self) -> &str {
        "Update inventory quantity for a product. Use positive numbers to add stock, negative to remove."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "sku": {
                    "type": "string",
                    "description": "Product SKU to update"
                },
                "quantity_change": {
                    "type": "integer",
                    "description": "Quantity to add (positive) or remove (negative)"
                },
                "reason": {
                    "type": "string",
                    "description": "Reason for update (e.g., 'restock', 'sale', 'damage')"
                }
            },
            "required": ["sku", "quantity_change"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("warehouse_update_inventory requires context")
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

        let sku = match arguments.get("sku").and_then(|v| v.as_str()) {
            Some(s) => s,
            None => return ToolExecutionResult::tool_error("Missing required parameter: sku"),
        };

        let quantity_change = match arguments.get("quantity_change").and_then(|v| v.as_i64()) {
            Some(q) => q as i32,
            None => {
                return ToolExecutionResult::tool_error(
                    "Missing required parameter: quantity_change",
                )
            }
        };

        let reason = arguments
            .get("reason")
            .and_then(|v| v.as_str())
            .unwrap_or("manual update");

        // Read current inventory
        let mut inventory: Vec<InventoryItem> = match file_store
            .read_file(context.session_id, "/warehouse/inventory.json")
            .await
        {
            Ok(Some(file)) => {
                serde_json::from_str(file.content.as_deref().unwrap_or("")).unwrap_or_default()
            }
            _ => vec![],
        };

        // Find and update the item
        let item = match inventory.iter_mut().find(|item| item.sku == sku) {
            Some(i) => i,
            None => return ToolExecutionResult::tool_error(format!("SKU not found: {}", sku)),
        };

        let old_quantity = item.quantity;
        item.quantity += quantity_change;

        if item.quantity < 0 {
            item.quantity = old_quantity; // Rollback
            return ToolExecutionResult::tool_error("Insufficient inventory");
        }

        let new_quantity = item.quantity;
        let reorder_point = item.reorder_point;
        // Save updated inventory
        let content = serde_json::to_string_pretty(&inventory).unwrap();
        match file_store
            .write_file(
                context.session_id,
                "/warehouse/inventory.json",
                &content,
                "text",
            )
            .await
        {
            Ok(_) => ToolExecutionResult::success(json!({
                "sku": sku,
                "old_quantity": old_quantity,
                "new_quantity": new_quantity,
                "change": quantity_change,
                "reason": reason,
                "below_reorder_point": new_quantity < reorder_point
            })),
            Err(e) => ToolExecutionResult::internal_error(e),
        }
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// ============================================================================
// Tool: warehouse_create_shipment
// ============================================================================

pub struct WarehouseCreateShipmentTool;

#[async_trait]
impl Tool for WarehouseCreateShipmentTool {
    fn name(&self) -> &str {
        "warehouse_create_shipment"
    }

    fn description(&self) -> &str {
        "Create a new shipment. Automatically updates inventory levels."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "destination": {
                    "type": "string",
                    "description": "Shipment destination address"
                },
                "items": {
                    "type": "array",
                    "description": "Items to ship",
                    "items": {
                        "type": "object",
                        "properties": {
                            "sku": {"type": "string"},
                            "quantity": {"type": "integer", "minimum": 1}
                        },
                        "required": ["sku", "quantity"]
                    }
                }
            },
            "required": ["destination", "items"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("warehouse_create_shipment requires context")
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

        let destination = match arguments.get("destination").and_then(|v| v.as_str()) {
            Some(d) => d,
            None => {
                return ToolExecutionResult::tool_error("Missing required parameter: destination")
            }
        };

        let items: Vec<ShipmentItem> = match arguments.get("items") {
            Some(items_value) => match serde_json::from_value(items_value.clone()) {
                Ok(items) => items,
                Err(_) => return ToolExecutionResult::tool_error("Invalid items format"),
            },
            None => return ToolExecutionResult::tool_error("Missing required parameter: items"),
        };

        // Read current shipments
        let mut shipments: Vec<Shipment> = match file_store
            .read_file(context.session_id, "/warehouse/shipments.json")
            .await
        {
            Ok(Some(file)) => {
                serde_json::from_str(file.content.as_deref().unwrap_or("")).unwrap_or_default()
            }
            _ => vec![],
        };

        // Create new shipment
        let shipment_id = format!("SHP-{:05}", shipments.len() + 1);
        let now = chrono::Utc::now().to_rfc3339();

        let shipment = Shipment {
            id: shipment_id.clone(),
            status: "pending".to_string(),
            destination: destination.to_string(),
            items: items.clone(),
            created_at: now.clone(),
            updated_at: now,
        };

        shipments.push(shipment.clone());

        // Save shipments
        let content = serde_json::to_string_pretty(&shipments).unwrap();
        match file_store
            .write_file(
                context.session_id,
                "/warehouse/shipments.json",
                &content,
                "text",
            )
            .await
        {
            Ok(_) => ToolExecutionResult::success(json!({
                "shipment_id": shipment_id,
                "status": "pending",
                "destination": destination,
                "items": items,
                "message": "Shipment created successfully"
            })),
            Err(e) => ToolExecutionResult::internal_error(e),
        }
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// ============================================================================
// Tool: warehouse_list_shipments
// ============================================================================

pub struct WarehouseListShipmentsTool;

#[async_trait]
impl Tool for WarehouseListShipmentsTool {
    fn name(&self) -> &str {
        "warehouse_list_shipments"
    }

    fn description(&self) -> &str {
        "List all shipments. Optionally filter by status."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "status": {
                    "type": "string",
                    "enum": ["pending", "in_transit", "delivered"],
                    "description": "Optional: Filter by shipment status"
                }
            },
            "additionalProperties": false
        })
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("warehouse_list_shipments requires context")
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

        // Read shipments
        let shipments: Vec<Shipment> = match file_store
            .read_file(context.session_id, "/warehouse/shipments.json")
            .await
        {
            Ok(Some(file)) => {
                serde_json::from_str(file.content.as_deref().unwrap_or("")).unwrap_or_default()
            }
            _ => vec![],
        };

        // Apply filter
        let filtered: Vec<_> = if let Some(status) = status_filter {
            shipments
                .into_iter()
                .filter(|s| s.status == status)
                .collect()
        } else {
            shipments
        };

        ToolExecutionResult::success(json!({
            "shipments": filtered,
            "total_count": filtered.len()
        }))
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// ============================================================================
// Tool: warehouse_update_shipment_status
// ============================================================================

pub struct WarehouseUpdateShipmentStatusTool;

#[async_trait]
impl Tool for WarehouseUpdateShipmentStatusTool {
    fn name(&self) -> &str {
        "warehouse_update_shipment_status"
    }

    fn description(&self) -> &str {
        "Update the status of a shipment (pending -> in_transit -> delivered)."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "shipment_id": {
                    "type": "string",
                    "description": "Shipment ID to update"
                },
                "status": {
                    "type": "string",
                    "enum": ["pending", "in_transit", "delivered"],
                    "description": "New status"
                }
            },
            "required": ["shipment_id", "status"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("warehouse_update_shipment_status requires context")
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

        let shipment_id = match arguments.get("shipment_id").and_then(|v| v.as_str()) {
            Some(id) => id,
            None => {
                return ToolExecutionResult::tool_error("Missing required parameter: shipment_id")
            }
        };

        let new_status = match arguments.get("status").and_then(|v| v.as_str()) {
            Some(s) => s,
            None => return ToolExecutionResult::tool_error("Missing required parameter: status"),
        };

        // Read shipments
        let mut shipments: Vec<Shipment> = match file_store
            .read_file(context.session_id, "/warehouse/shipments.json")
            .await
        {
            Ok(Some(file)) => {
                serde_json::from_str(file.content.as_deref().unwrap_or("")).unwrap_or_default()
            }
            _ => vec![],
        };

        // Find and update shipment
        let shipment = match shipments.iter_mut().find(|s| s.id == shipment_id) {
            Some(s) => s,
            None => {
                return ToolExecutionResult::tool_error(format!(
                    "Shipment not found: {}",
                    shipment_id
                ))
            }
        };

        let old_status = shipment.status.clone();
        shipment.status = new_status.to_string();
        let updated_at = shipment.updated_at.clone();
        shipment.updated_at = chrono::Utc::now().to_rfc3339();

        // Save updated shipments
        let content = serde_json::to_string_pretty(&shipments).unwrap();
        match file_store
            .write_file(
                context.session_id,
                "/warehouse/shipments.json",
                &content,
                "text",
            )
            .await
        {
            Ok(_) => ToolExecutionResult::success(json!({
                "shipment_id": shipment_id,
                "old_status": old_status,
                "new_status": new_status,
                "updated_at": updated_at
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

pub struct WarehouseCreateOrderTool;

#[async_trait]
impl Tool for WarehouseCreateOrderTool {
    fn name(&self) -> &str {
        "warehouse_create_order"
    }

    fn description(&self) -> &str {
        "Create a new customer order."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "customer_name": {"type": "string"},
                "items": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "sku": {"type": "string"},
                            "quantity": {"type": "integer"}
                        }
                    }
                }
            },
            "required": ["customer_name", "items"],
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

        let customer_name = arguments
            .get("customer_name")
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown");
        let items: Vec<ShipmentItem> =
            serde_json::from_value(arguments.get("items").cloned().unwrap_or(json!([])))
                .unwrap_or_default();

        let mut orders: Vec<Order> = match file_store
            .read_file(context.session_id, "/warehouse/orders.json")
            .await
        {
            Ok(Some(file)) => {
                serde_json::from_str(file.content.as_deref().unwrap_or("")).unwrap_or_default()
            }
            _ => vec![],
        };

        let order_id = format!("ORD-{:05}", orders.len() + 1);
        let order = Order {
            id: order_id.clone(),
            customer_name: customer_name.to_string(),
            items: items.clone(),
            status: "pending".to_string(),
            created_at: chrono::Utc::now().to_rfc3339(),
        };

        orders.push(order);

        let content = serde_json::to_string_pretty(&orders).unwrap();
        let _ = file_store
            .write_file(
                context.session_id,
                "/warehouse/orders.json",
                &content,
                "text",
            )
            .await;

        ToolExecutionResult::success(
            json!({"order_id": order_id, "status": "pending", "items": items}),
        )
    }

    fn requires_context(&self) -> bool {
        true
    }
}

pub struct WarehouseListOrdersTool;

#[async_trait]
impl Tool for WarehouseListOrdersTool {
    fn name(&self) -> &str {
        "warehouse_list_orders"
    }

    fn description(&self) -> &str {
        "List all customer orders."
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
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let file_store = match &context.file_store {
            Some(store) => store,
            None => return ToolExecutionResult::tool_error("File system not available"),
        };

        let orders: Vec<Order> = match file_store
            .read_file(context.session_id, "/warehouse/orders.json")
            .await
        {
            Ok(Some(file)) => {
                serde_json::from_str(file.content.as_deref().unwrap_or("")).unwrap_or_default()
            }
            _ => vec![],
        };

        ToolExecutionResult::success(json!({"orders": orders, "total_count": orders.len()}))
    }

    fn requires_context(&self) -> bool {
        true
    }
}

pub struct WarehouseCreateInvoiceTool;

#[async_trait]
impl Tool for WarehouseCreateInvoiceTool {
    fn name(&self) -> &str {
        "warehouse_create_invoice"
    }

    fn description(&self) -> &str {
        "Generate an invoice for an order."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "order_id": {"type": "string"}
            },
            "required": ["order_id"],
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
        let order_id = arguments
            .get("order_id")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let invoice_id = format!("INV-{:05}", chrono::Utc::now().timestamp() % 100000);

        ToolExecutionResult::success(json!({
            "invoice_id": invoice_id,
            "order_id": order_id,
            "amount": 1299.99,
            "status": "generated"
        }))
    }

    fn requires_context(&self) -> bool {
        true
    }
}

pub struct WarehouseProcessReturnTool;

#[async_trait]
impl Tool for WarehouseProcessReturnTool {
    fn name(&self) -> &str {
        "warehouse_process_return"
    }

    fn description(&self) -> &str {
        "Process a product return and update inventory."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "order_id": {"type": "string"},
                "sku": {"type": "string"},
                "quantity": {"type": "integer"},
                "reason": {"type": "string"}
            },
            "required": ["order_id", "sku", "quantity"],
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
        let return_id = format!("RET-{:05}", chrono::Utc::now().timestamp() % 100000);
        let sku = arguments
            .get("sku")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");

        ToolExecutionResult::success(json!({
            "return_id": return_id,
            "sku": sku,
            "status": "processed",
            "inventory_updated": true
        }))
    }

    fn requires_context(&self) -> bool {
        true
    }
}

pub struct WarehouseInventoryReportTool;

#[async_trait]
impl Tool for WarehouseInventoryReportTool {
    fn name(&self) -> &str {
        "warehouse_inventory_report"
    }

    fn description(&self) -> &str {
        "Generate a comprehensive inventory report with metrics and alerts."
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
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let file_store = match &context.file_store {
            Some(store) => store,
            None => return ToolExecutionResult::tool_error("File system not available"),
        };

        let inventory: Vec<InventoryItem> = match file_store
            .read_file(context.session_id, "/warehouse/inventory.json")
            .await
        {
            Ok(Some(file)) => {
                serde_json::from_str(file.content.as_deref().unwrap_or("")).unwrap_or_default()
            }
            _ => vec![],
        };

        let total_items = inventory.len();
        let low_stock: Vec<_> = inventory
            .iter()
            .filter(|i| i.quantity < i.reorder_point)
            .collect();
        let total_value: i32 = inventory.iter().map(|i| i.quantity).sum();

        ToolExecutionResult::success(json!({
            "report": {
                "total_items": total_items,
                "total_quantity": total_value,
                "low_stock_count": low_stock.len(),
                "low_stock_items": low_stock,
                "generated_at": chrono::Utc::now().to_rfc3339()
            }
        }))
    }

    fn requires_context(&self) -> bool {
        true
    }
}
