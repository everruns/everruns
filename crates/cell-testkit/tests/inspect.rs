//! The cell inspector against a live fleet: where a session is and what it
//! is doing, and a cell with a node down.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod support;

use everruns_cell::inspect::{Inspector, render_cells, render_session};
use support::Fleet;

#[tokio::test(flavor = "multi_thread")]
async fn the_inspector_finds_a_session_and_reports_every_cell() {
    let mut fleet = Fleet::start().await;
    fleet.executor(1);
    let session = fleet.create_in("chat", 1).await;
    fleet.say(&session, "hello").await;
    fleet.turns(&session, 1).await;

    // The SLO gauges the dashboard charts: the cell that ran the turn
    // reports its one-minute p99.
    let leader = fleet.leader(1).await;
    let addr = fleet
        .addrs(1)
        .into_iter()
        .find(|(n, _)| *n == leader)
        .unwrap()
        .1;
    let metrics = reqwest::get(format!("http://{addr}/metrics"))
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    let gauge = |name: &str| -> f64 {
        metrics
            .lines()
            .find(|l| l.starts_with(&format!("{name}{{")))
            .and_then(|l| l.rsplit(' ').next())
            .and_then(|v| v.parse().ok())
            .unwrap_or_else(|| panic!("no {name} in\n{metrics}"))
    };
    assert!(gauge("cell_slo_turn_p99_seconds") > 0.0);
    for name in [
        "cell_slo_ship_lag_p99_seconds",
        "cell_slo_ship_first_p99_seconds",
        "cell_slo_admit_p99_seconds",
    ] {
        assert!(gauge(name) >= 0.0);
    }

    let inspector = Inspector::new(fleet.topology().read().unwrap()).unwrap();
    let id = everruns::SessionId::parse(&session).unwrap();
    let placed = inspector.placed(id).await.expect("the directory places it");
    assert_eq!(placed.cell, 1);
    let report = inspector.session(1, id).await.expect("cell 1 holds it");
    assert_eq!(report.cell, 1);
    assert!(report.turns_completed >= 1, "{report:?}");
    assert!(!report.tail.is_empty());
    let text = render_session(id, Some(placed), Some(&report));
    assert!(text.contains("directory: cell 1"), "{text}");
    assert!(text.contains("turn.completed"), "{text}");

    // The directory spreads its shards over the cells: every session is
    // found wherever its shard lives (a load run showed sessions whose
    // shard sat in cell 1 as unknown).
    let addr = fleet.addrs(0)[0].1.clone();
    fleet
        .until("directory shards reach cell 1", || {
            let addr = addr.clone();
            async move {
                let registry: serde_json::Value =
                    reqwest::get(format!("http://{addr}/cell/registry"))
                        .await
                        .unwrap()
                        .json()
                        .await
                        .unwrap();
                registry["shards"].as_object().is_some_and(|s| s.len() > 64)
            }
        })
        .await;
    for n in 0..8 {
        let cell = n % 2;
        let other = fleet.create_in("chat", cell).await;
        let id = everruns::SessionId::parse(&other).unwrap();
        let placed = inspector.placed(id).await;
        assert_eq!(placed.map(|p| p.cell), Some(cell), "{other}");
    }

    let reports = inspector.reports().await;
    assert_eq!(reports.keys().copied().collect::<Vec<_>>(), vec![0, 1]);
    let text = render_cells(&reports);
    assert!(!text.contains("leader NONE"), "{text}");
    assert!(!text.contains("NO ANSWER"), "{text}");

    // A follower of cell 0 goes: the cell still has a leader, and the
    // inspector names the node that does not answer.
    let leader = fleet.leader(0).await;
    let follower = (1..=3).find(|n| *n != leader).unwrap();
    fleet.kill_node(0, follower);
    let text = render_cells(&inspector.reports().await);
    assert!(
        text.contains(&format!("node {follower}: NO ANSWER")),
        "{text}"
    );
    assert!(text.contains("cell 0: leader"), "{text}");
    assert!(!text.contains("cell 0: leader NONE"), "{text}");
    fleet.check();
}

#[test]
fn a_session_mid_move_is_listed_with_the_directory_view() {
    use everruns_cell::inspect::{CellReport, MoveView, render_moves};
    use everruns_cell::state::{Placed, Placement};
    let session = everruns::SessionId::new();
    let report = CellReport {
        cell: 0,
        node: 1,
        leader: Some(1),
        incarnation: 1,
        applied: 10,
        shipped: Some(10),
        unshipped_ms: None,
        last_snapshot_s: 3,
        draining: true,
        sessions: 1,
        active_turns: 0,
        queued_inputs: 0,
        feed_pending: 0,
        rss_mb: 10,
        moves_total: 1,
        moves: vec![MoveView {
            session,
            placement: Placement::Frozen,
            generation: 4,
            moving_to: Some(1),
        }],
    };
    let reports = [(0, vec![(1, Some(report))])].into_iter().collect();
    let placed = [(
        session,
        Some(Placed {
            cell: 0,
            generation: 4,
        }),
    )]
    .into_iter()
    .collect();
    let text = render_moves(&reports, &placed);
    assert!(
        text.contains("Frozen at generation 4, to 1; directory says cell 0 at generation 4"),
        "{text}"
    );
    assert!(render_cells(&reports).contains("DRAINING"));
}
