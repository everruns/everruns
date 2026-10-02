use serde_json::Value;
use serve::prelude::*;

/// Look up the shipping status of an order.
#[tool]
async fn order_status(cx: &Cx, order_id: String) -> Result<Value> {
    cx.progress(format!("looking up {order_id}")).await;
    // A real app calls its order system here.
    Ok(json!({ "order_id": order_id, "status": "shipped" }))
}
