use serve::prelude::*;
use serve::sim;

/// Answers revenue questions from the warehouse.
#[agent]
fn analyst() -> Agent {
    Agent::builder()
        // Resolved by the host's model gateway; the app holds no provider keys.
        .model("anthropic/claude-sonnet-5")
        .instructions(md!("instructions.md"))
        .offline(offline_demo())
        .build()
}

/// What the agent does when no model gateway is configured: a scripted demo
/// conversation, so `dev` and the evals run with no network or keys.
///
/// 1. A cheap, date-filtered query (no approval).
/// 2. A full scan, which pauses for a person's approval.
/// 3. A delegation to the reviewer subagent.
fn offline_demo() -> serve::sim::LlmSimConfig {
    sim::script([
        sim::call("activate_skill", json!({ "name": "sql-style" })),
        sim::call(
            "run_sql",
            json!({ "sql": "SELECT SUM(o.amount_cents - COALESCE(r.amount_cents, 0)) / 100.0 AS net_revenue \
                            FROM orders o LEFT JOIN refunds r ON r.order_id = o.id \
                            WHERE o.placed_at >= '2026-09-14' AND o.placed_at < '2026-09-21'" }),
        ),
        sim::reply(
            "Last week's revenue, net of refunds, is in the run_sql result above. (Offline demo reply.)",
        ),
        sim::call("run_sql", json!({ "sql": "SELECT * FROM orders" })),
        sim::reply("Here is every order on record. (Offline demo reply.)"),
        sim::call(
            "ask_reviewer",
            json!({ "task": "Check that the weekly revenue query subtracts refunds and uses a half-open date range." }),
        ),
        sim::reply("The reviewer confirmed the query. (Offline demo reply.)"),
    ])
}
