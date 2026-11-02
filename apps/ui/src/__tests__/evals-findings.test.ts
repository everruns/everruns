// Finding helpers: the Inspector's severity wins over the detector's default,
// similar findings group under their cluster, and evidence numbers read as
// labelled rows without session content or internal flags.

import {
  evidenceRows,
  findingKind,
  findingLevel,
  groupFindings,
} from "@/components/evals/findings";
import type { HealthIssue } from "@/lib/api/types";

function finding(
  id: string,
  code: string,
  investigation?: Partial<NonNullable<HealthIssue["investigation"]>>,
  detected = "2026-11-01T00:00:00Z",
): HealthIssue {
  return {
    id,
    code,
    status: "open",
    title: id,
    body: "",
    first_detected_at: detected,
    investigation: investigation ? { status: "done", ...investigation } : null,
  } as HealthIssue;
}

describe("findings", () => {
  it("uses the Inspector's severity when it judged one", () => {
    expect(findingLevel(finding("a", "session.agent_stuck"))).toBe("medium");
    expect(findingLevel(finding("b", "session.agent_stuck", { severity: "high" }))).toBe("high");
    expect(findingLevel(finding("c", "session.suspected_abuse", { severity: "low" }))).toBe("low");
    expect(findingKind(finding("d", "schedule.runaway"))).toBe("Runaway work");
  });

  it("groups findings of one cluster under the loudest, newest one", () => {
    const groups = groupFindings([
      finding("old", "session.agent_stuck", { cluster_id: "k1" }, "2026-10-30T00:00:00Z"),
      finding("new", "session.agent_stuck", { cluster_id: "k1" }, "2026-11-01T00:00:00Z"),
      finding("loud", "session.agent_stuck", { cluster_id: "k1", severity: "high" }),
      finding("alone", "session.spend_spike"),
      finding("not-a-finding", "slack.permissions"),
    ]);
    expect(groups.map((g) => [g.lead.id, g.similar])).toEqual([
      ["loud", 2],
      ["alone", 0],
    ]);
  });

  it("reads evidence as labelled rows and skips flags and nested values", () => {
    expect(
      evidenceRows({
        probability: 0.9731,
        calibrated: true,
        reason: "flagged",
        last_hour_usd: 2.5,
        cached_percent: 2.4,
        some_new_number: 7,
        nested: { a: 1 },
      }),
    ).toEqual([
      { label: "Model's confidence", value: "0.97" },
      { label: "Read because", value: "a detector flagged it" },
      { label: "Last hour", value: "$2.50" },
      { label: "Cache hits", value: "2%" },
      { label: "Some new number", value: "7" },
    ]);
    expect(evidenceRows(null)).toEqual([]);
  });
});
