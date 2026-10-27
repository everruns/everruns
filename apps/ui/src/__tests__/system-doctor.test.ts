import type { DoctorCheck, DoctorReport } from "@/lib/api/types";
import {
  doctorReportText,
  doctorVerdict,
  groupDoctorChecks,
} from "@/lib/system-doctor";

function check(
  id: string,
  status: DoctorCheck["status"],
  area: DoctorCheck["area"],
): DoctorCheck {
  return {
    id,
    area,
    title: id,
    status,
    summary: `${id} summary`,
    observed_at: "2026-10-27T08:00:00Z",
  };
}

function report(
  checks: DoctorCheck[],
  status: DoctorReport["status"],
): DoctorReport {
  return {
    status,
    checked_at: "2026-10-27T08:00:00Z",
    version: "0.50.0",
    grade: "prod",
    auth_mode: "full",
    setup: ["PostgreSQL 17.2", "2 API servers"],
    checks,
  };
}

describe("system doctor presentation", () => {
  it("states the verdict from problems and warnings, ignoring notes", () => {
    expect(doctorVerdict(report([check("a", "ok", "database")], "ok"))).toBe(
      "Everything is set up and working.",
    );
    expect(
      doctorVerdict(
        report(
          [check("a", "warning", "messaging"), check("b", "info", "storage")],
          "warning",
        ),
      ),
    ).toBe("Everything works, with 1 warning to look at.");
    expect(
      doctorVerdict(
        report(
          [
            check("a", "problem", "database"),
            check("b", "problem", "workers"),
            check("c", "warning", "models"),
          ],
          "problem",
        ),
      ),
    ).toBe("2 problems need attention, and 1 warning.");
  });

  it("lifts problems and warnings out and groups the rest by area in display order", () => {
    const { attention, byArea } = groupDoctorChecks([
      check("db.problem", "problem", "database"),
      check("storage.ok", "ok", "storage"),
      check("plane.ok", "ok", "control_plane"),
      check("nats.note", "info", "messaging"),
      check("nats.warn", "warning", "messaging"),
    ]);
    expect(attention.map((c) => c.id)).toEqual(["db.problem", "nats.warn"]);
    expect(byArea.map((group) => group.area)).toEqual([
      "control_plane",
      "messaging",
      "storage",
    ]);
  });

  it("writes a plain-text report with the why and fix of each check", () => {
    const failing = {
      ...check("database.migrations", "problem", "database"),
      why: "W",
      fix: "F",
    };
    const text = doctorReportText(report([failing], "problem"));
    expect(text).toContain("Everruns 0.50.0 system doctor: problem");
    expect(text).toContain("Setup: PostgreSQL 17.2, 2 API servers");
    expect(text).toContain(
      "[problem] database.migrations (database.migrations): database.migrations summary",
    );
    expect(text).toContain("  Why: W");
    expect(text).toContain("  Fix: F");
  });
});
