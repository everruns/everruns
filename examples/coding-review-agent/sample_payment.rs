pub fn refund(order_total_cents: u64, requested_cents: u64) -> u64 {
    requested_cents.min(order_total_cents)
}

pub fn issue_refund(order_total_cents: u64, requested_cents: u64) -> String {
    let amount = refund(order_total_cents, requested_cents);
    format!("refund issued for {amount} cents")
}
