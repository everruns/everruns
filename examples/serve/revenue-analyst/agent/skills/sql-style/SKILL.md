---
name: sql-style
description: How to query the shop warehouse (tables, joins, date ranges).
---

# Warehouse SQL style

Tables:

- `orders(id INTEGER, placed_at TEXT /* YYYY-MM-DD */, amount_cents INTEGER)`
- `refunds(order_id INTEGER, refunded_at TEXT, amount_cents INTEGER)`

Rules:

1. Money is stored in cents. Divide by `100.0` only in the final projection.
2. Net revenue = `SUM(o.amount_cents - COALESCE(r.amount_cents, 0))` with
   `LEFT JOIN refunds r ON r.order_id = o.id`.
3. Date ranges are half-open: `placed_at >= :start AND placed_at < :end`.
4. Always filter by date. A query without `WHERE` scans every order and needs
   a person's approval.
