"use client";

// Open findings from detectors and classifiers, loudest first. Used by the
// Evals Findings tab, the overview's "Needs a look" panel and an agent's Evals
// tab. Each links to its health issue, which carries the evidence.

import Link from "next/link";
import { useLocale, useUiTranslations } from "@/providers/locale-provider";
import { formatRelativeTime } from "@/lib/formatting";
import { cn } from "@/lib/utils";
import type { HealthIssue } from "@/lib/api/types";
import { findingSeverity, sortFindings } from "../findings";

export function SeverityTag({ issue }: { issue: HealthIssue }) {
  const ui = useUiTranslations();
  const high = findingSeverity(issue) === "high";
  return (
    <span
      className={cn(
        "inline-flex shrink-0 items-center px-1.5 py-0.5 text-[11px] font-medium",
        high ? "bg-destructive text-destructive-foreground" : "bg-warning/15 text-foreground",
      )}
    >
      {high ? ui("High") : ui("Medium")}
    </span>
  );
}

export function FindingsList({
  issues,
  limit,
  emptyText,
  showTime = false,
}: {
  issues: HealthIssue[];
  limit?: number;
  emptyText: string;
  showTime?: boolean;
}) {
  const { displayLocale } = useLocale();
  const findings = sortFindings(issues).slice(0, limit);
  if (findings.length === 0) {
    return <p className="px-4 py-6 text-sm text-muted-foreground">{emptyText}</p>;
  }
  return (
    <ul className="divide-y">
      {findings.map((issue) => (
        <li key={issue.id}>
          <Link
            href={issue.href}
            className="block space-y-1 px-4 py-3 transition-colors hover:bg-muted/50"
          >
            <span className="flex items-start gap-2">
              <SeverityTag issue={issue} />
              <span className="text-sm font-semibold">{issue.title}</span>
            </span>
            <span className="block text-xs text-muted-foreground">
              {[
                issue.agent_name,
                showTime ? formatRelativeTime(issue.first_detected_at, displayLocale) : null,
              ]
                .filter(Boolean)
                .join(" · ")}
            </span>
          </Link>
        </li>
      ))}
    </ul>
  );
}
