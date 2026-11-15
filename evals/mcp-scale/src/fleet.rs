// Included into `fixture.rs`: the many-call task's generated ticket fleet.

fn list_tickets_tool() -> Value {
    tool(
                "list_tickets",
                "List tickets, optionally filtered by status and priority. Returns {\"items\": [{ticket_id, service, priority, status}], \"next_cursor\": string or null}, 20 per page; pass next_cursor back as cursor for the next page. SLA state is only on get_ticket.",
                object(
                    json!({"status":string("Optional ticket state.", &["open", "closed"]), "priority":string("Optional priority.", &["P1", "P2", "P3"]), "cursor":string("next_cursor from the previous page.", &[])}),
                    &[],
                ),
            )
}

/// `get_ticket` and `list_tickets` on the telemetry server for fleet tickets.
fn fleet_call(name: &str, args: &Value) -> Value {
    if name == "list_tickets" {
        return list_tickets(args);
    }
    args["ticket_id"]
        .as_str()
        .and_then(|id| fleet_tickets().into_iter().find(|t| t["ticket_id"] == id))
        .unwrap_or_else(|| json!({"error":"ticket not found"}))
}

const TICKET_PAGE: usize = 20;
const TICKET_SERVICES: [&str; 8] = [
    "billing-api",
    "checkout",
    "identity",
    "search",
    "storage",
    "messaging",
    "fraud",
    "edge",
];

/// The many-call task's data: 120 generated tickets. Listing shows service,
/// priority and status; whether a ticket breached its SLA is only on
/// `get_ticket`, so answering takes one detail call per open P1 ticket.
pub fn fleet_tickets() -> Vec<Value> {
    let mut seed: u64 = 0x5eed_7ac4;
    let mut next = move |modulo: u64| {
        seed = seed
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (seed >> 33) % modulo
    };
    (0..120)
        .map(|index| {
            let service = TICKET_SERVICES[next(TICKET_SERVICES.len() as u64) as usize];
            let priority = ["P1", "P1", "P2", "P3"][next(4) as usize];
            let status = if next(5) == 0 { "closed" } else { "open" };
            // Breach odds out of 10 per service, so the ranking has clear leaders.
            const BREACH_ODDS: [u64; 8] = [3, 6, 1, 8, 10, 2, 4, 1];
            let odds = BREACH_ODDS[TICKET_SERVICES
                .iter()
                .position(|s| *s == service)
                .unwrap_or(0)];
            let breached = next(10) < odds;
            json!({
                "ticket_id": format!("TCK-{}", 1001 + index),
                "service": service,
                "priority": priority,
                "status": status,
                "owner": format!("oncall-{service}"),
                "sla_hours": if priority == "P1" { 4 } else { 24 },
                "sla_breached": breached,
            })
        })
        .collect()
}

/// Open P1 tickets that breached their SLA, counted per service, most first
/// (ties by name). The many-call task's expected answer.
pub fn sla_breach_ranking() -> Vec<(String, usize)> {
    let mut counts = std::collections::BTreeMap::<String, usize>::new();
    for ticket in fleet_tickets() {
        if ticket["status"] == "open"
            && ticket["priority"] == "P1"
            && ticket["sla_breached"] == true
        {
            *counts
                .entry(ticket["service"].as_str().unwrap_or_default().to_string())
                .or_default() += 1;
        }
    }
    let mut ranking: Vec<_> = counts.into_iter().collect();
    ranking.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    ranking
}

fn list_tickets(args: &Value) -> Value {
    let matching: Vec<Value> = fleet_tickets()
        .into_iter()
        .filter(|t| args["status"].as_str().is_none_or(|s| t["status"] == s))
        .filter(|t| args["priority"].as_str().is_none_or(|p| t["priority"] == p))
        .map(|t| json!({"ticket_id":t["ticket_id"], "service":t["service"], "priority":t["priority"], "status":t["status"]}))
        .collect();
    let start = args["cursor"]
        .as_str()
        .and_then(|cursor| cursor.strip_prefix("page-"))
        .and_then(|n| n.parse::<usize>().ok())
        .unwrap_or(0);
    let end = (start + TICKET_PAGE).min(matching.len());
    let next_cursor = (end < matching.len()).then(|| format!("page-{end}"));
    json!({
        "items": matching.get(start..end).unwrap_or_default(),
        "next_cursor": next_cursor,
        "total": matching.len(),
    })
}

#[cfg(test)]
mod fleet_ticket_tests {
    use super::*;

    #[test]
    fn ranking_has_a_clear_top_three_and_needs_many_detail_calls() {
        let ranking = sla_breach_ranking();
        eprintln!("{ranking:?}");
        assert!(ranking.len() >= 4);
        assert!(
            ranking[0].1 > ranking[1].1
                && ranking[1].1 > ranking[2].1
                && ranking[2].1 > ranking[3].1
        );
        let open_p1 = fleet_tickets()
            .into_iter()
            .filter(|t| t["status"] == "open" && t["priority"] == "P1")
            .count();
        assert!(open_p1 >= 30, "{open_p1} detail calls");
    }

    #[test]
    fn list_tickets_pages_and_hides_sla_state() {
        let first = list_tickets(&json!({"status":"open","priority":"P1"}));
        assert_eq!(first["items"].as_array().map(Vec::len), Some(TICKET_PAGE));
        assert!(first["items"][0].get("sla_breached").is_none());
        let mut seen = first["items"].as_array().unwrap().len();
        let mut cursor = first["next_cursor"].as_str().map(str::to_owned);
        while let Some(c) = cursor {
            let page = list_tickets(&json!({"status":"open","priority":"P1","cursor":c}));
            seen += page["items"].as_array().unwrap().len();
            cursor = page["next_cursor"].as_str().map(str::to_owned);
        }
        assert_eq!(seen as u64, first["total"].as_u64().unwrap());
    }

    #[test]
    fn get_ticket_reveals_sla_state_on_the_telemetry_server_only() {
        let mut telemetry = initial_state("telemetry");
        let detail = call(
            &mut telemetry,
            "get_ticket",
            &json!({"ticket_id":"TCK-1001"}),
        );
        assert!(detail["sla_breached"].is_boolean());
        let ops = call(
            &mut telemetry,
            "get_ticket",
            &json!({"ticket_id":"OPS-8421"}),
        );
        assert_eq!(ops["owner"], "Ava Li");
        let mut peer = initial_state("alpha");
        assert!(
            call(&mut peer, "get_ticket", &json!({"ticket_id":"TCK-1001"}))["error"].is_string()
        );
    }
}
