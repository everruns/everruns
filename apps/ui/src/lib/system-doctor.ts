// Presentation rules for the system doctor report (System → Health).

import type {
  CheckArea,
  CheckStatus,
  DoctorCheck,
  DoctorReport,
} from "@/lib/api/types";

export const CHECK_AREA_LABELS: Record<CheckArea, string> = {
  control_plane: "Control plane",
  database: "Database",
  workers: "Workers",
  security: "Security",
  messaging: "Messaging",
  storage: "File storage",
  observability: "Observability",
  models: "Models",
};

export const CHECK_STATUS_LABELS: Record<CheckStatus, string> = {
  ok: "OK",
  info: "Note",
  warning: "Warning",
  problem: "Problem",
};

/** The banner's one line: what the report means for the operator. */
export function doctorVerdict(report: DoctorReport): string {
  const problems = report.checks.filter(
    (check) => check.status === "problem",
  ).length;
  const warnings = report.checks.filter(
    (check) => check.status === "warning",
  ).length;
  if (problems > 0) {
    return `${plural(problems, "problem needs", "problems need")} attention${
      warnings > 0 ? `, and ${plural(warnings, "warning", "warnings")}` : ""
    }.`;
  }
  if (warnings > 0) {
    return `Everything works, with ${plural(warnings, "warning", "warnings")} to look at.`;
  }
  return "Everything is set up and working.";
}

/** Checks split the way the page shows them: what needs a look, then the rest by area. */
export function groupDoctorChecks(checks: DoctorCheck[]): {
  attention: DoctorCheck[];
  byArea: Array<{ area: CheckArea; checks: DoctorCheck[] }>;
} {
  const attention = checks.filter(
    (check) => check.status === "problem" || check.status === "warning",
  );
  const areas = new Map<CheckArea, DoctorCheck[]>();
  for (const check of checks) {
    if (check.status === "problem" || check.status === "warning") continue;
    const list = areas.get(check.area) ?? [];
    list.push(check);
    areas.set(check.area, list);
  }
  const order = Object.keys(CHECK_AREA_LABELS) as CheckArea[];
  const byArea = order
    .filter((area) => areas.has(area))
    .map((area) => ({ area, checks: areas.get(area) ?? [] }));
  return { attention, byArea };
}

/** Plain text for pasting into an issue or a chat with support. */
export function doctorReportText(report: DoctorReport): string {
  const lines = [
    `Everruns ${report.version} system doctor: ${report.status}`,
    `Checked ${report.checked_at} · grade ${report.grade} · auth ${report.auth_mode}`,
  ];
  if (report.setup.length > 0) lines.push(`Setup: ${report.setup.join(", ")}`);
  lines.push("");
  for (const check of report.checks) {
    lines.push(
      `[${check.status}] ${check.title} (${check.id}): ${check.summary}`,
    );
    if (check.why) lines.push(`  Why: ${check.why}`);
    if (check.fix) lines.push(`  Fix: ${check.fix}`);
  }
  return lines.join("\n");
}

function plural(count: number, one: string, many: string): string {
  return count === 1 ? `1 ${one}` : `${count} ${many}`;
}
