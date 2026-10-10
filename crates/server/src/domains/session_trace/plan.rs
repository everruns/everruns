// Which rows a turn shows: batching of repeated calls and elision of long turns.
//
// Decision: five or more consecutive calls of one tool become one batch row,
// and a turn with more than 40 rows after batching shows its first and last
// 12 with a gap between them; the gap expands through the steps page. The
// handoff started at 200/50, but 20 such turns made the first page about
// 800 KB, over the 250 KB bar; at 40/12 a page of 20 turns stays near 500 rows.

use crate::storage::TraceRunRow;

/// Consecutive calls of one tool that fold into a batch.
pub const BATCH_MIN: i64 = 5;
/// Rows a turn shows in full.
pub const LONG_TURN_ROWS: usize = 40;
/// Rows kept at each end of a longer turn.
pub const LONG_TURN_EDGE: usize = 12;

/// A row before its step data is loaded.
#[derive(Debug, Clone, PartialEq)]
pub enum PlannedRow {
    Step {
        step: i32,
    },
    Batch(TraceRunRow),
    Gap {
        first_step: i32,
        last_step: i32,
        count: i64,
        errors: i64,
    },
}

struct Sized {
    row: PlannedRow,
    first_step: i32,
    last_step: i32,
    count: i64,
    errors: i64,
}

/// Rows for one turn's runs, in step order.
pub fn plan_turn(runs: &[TraceRunRow]) -> Vec<PlannedRow> {
    let mut rows: Vec<Sized> = Vec::new();
    for run in runs {
        if run.kind == "tool" && run.count >= BATCH_MIN {
            rows.push(Sized {
                row: PlannedRow::Batch(run.clone()),
                first_step: run.first_step,
                last_step: run.last_step,
                count: run.count,
                errors: run.failed,
            });
        } else {
            // A short run: each step is its own row. Failures are counted per
            // run, so they go on the run's last row; that keeps a gap's error
            // total exact without knowing which step failed.
            for step in run.first_step..=run.last_step {
                rows.push(Sized {
                    row: PlannedRow::Step { step },
                    first_step: step,
                    last_step: step,
                    count: 1,
                    errors: if step == run.last_step { run.failed } else { 0 },
                });
            }
        }
    }
    if rows.len() <= LONG_TURN_ROWS {
        return rows.into_iter().map(|r| r.row).collect();
    }
    let tail_start = rows.len() - LONG_TURN_EDGE;
    let hidden = &rows[LONG_TURN_EDGE..tail_start];
    let gap = PlannedRow::Gap {
        first_step: hidden.first().map_or(0, |r| r.first_step),
        last_step: hidden.last().map_or(0, |r| r.last_step),
        count: hidden.iter().map(|r| r.count).sum(),
        errors: hidden.iter().map(|r| r.errors).sum(),
    };
    let mut out: Vec<PlannedRow> = Vec::with_capacity(LONG_TURN_EDGE * 2 + 1);
    let mut iter = rows.into_iter();
    out.extend(iter.by_ref().take(LONG_TURN_EDGE).map(|r| r.row));
    out.push(gap);
    out.extend(iter.skip(tail_start - LONG_TURN_EDGE).map(|r| r.row));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn run(first: i32, last: i32, kind: &str, name: &str, failed: i64) -> TraceRunRow {
        TraceRunRow {
            turn_no: 1,
            first_step: first,
            last_step: last,
            kind: kind.to_string(),
            name: Some(name.to_string()),
            count: i64::from(last - first + 1),
            failed,
            running: 0,
            p50_ms: None,
            p95_ms: None,
            started_at: Utc::now(),
            ended_at: None,
        }
    }

    #[test]
    fn repeated_calls_fold_and_short_runs_stay_steps() {
        let rows = plan_turn(&[
            run(1, 1, "model", "m", 0),
            run(2, 1185, "tool", "web_fetch", 3),
            run(1186, 1189, "tool", "get_agent", 0),
            run(1190, 1190, "answer", "m", 0),
        ]);
        assert_eq!(rows.len(), 1 + 1 + 4 + 1);
        assert!(matches!(rows[1], PlannedRow::Batch(ref b) if b.count == 1184 && b.failed == 3));
        assert_eq!(rows[2], PlannedRow::Step { step: 1186 });
    }

    #[test]
    fn long_turns_keep_both_ends_and_count_the_middle() {
        // Alternating tools never batch: 1,000 single-step runs.
        let runs: Vec<_> = (1..=1000)
            .map(|i| {
                run(
                    i,
                    i,
                    "tool",
                    if i % 2 == 0 { "a" } else { "b" },
                    i64::from(i % 10 == 0),
                )
            })
            .collect();
        let rows = plan_turn(&runs);
        assert_eq!(rows.len(), LONG_TURN_EDGE * 2 + 1);
        assert_eq!(rows[0], PlannedRow::Step { step: 1 });
        assert_eq!(rows[LONG_TURN_EDGE - 1], PlannedRow::Step { step: 12 });
        assert_eq!(
            rows[LONG_TURN_EDGE],
            PlannedRow::Gap {
                first_step: 13,
                last_step: 988,
                count: 976,
                errors: 97,
            }
        );
        assert_eq!(rows[LONG_TURN_EDGE + 1], PlannedRow::Step { step: 989 });
        assert_eq!(rows.last(), Some(&PlannedRow::Step { step: 1000 }));
    }

    #[test]
    fn a_turn_at_the_limit_is_shown_whole() {
        let runs: Vec<_> = (1..=LONG_TURN_ROWS as i32)
            .map(|i| run(i, i, "model", "m", 0))
            .collect();
        assert_eq!(plan_turn(&runs).len(), LONG_TURN_ROWS);
    }
}
