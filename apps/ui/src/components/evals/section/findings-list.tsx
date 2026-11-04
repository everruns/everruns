"use client";

// Open findings from detectors and classifiers, loudest first. Used by the
// Evals Findings tab, the overview's "Needs a look" panel and an agent's Evals
// tab. Findings the Inspector filed under one cluster show as one row with a
// count of similar ones. Each links to its finding page (mockup 04).

import Link from "next/link";
import { useLocale, useUiTranslations } from "@/providers/locale-provider";
import { formatRelativeTime } from "@/lib/formatting";
import { cn } from "@/lib/utils";
import type { HealthIssue } from "@/lib/api/types";
import { findingLevel, groupFindings } from "../findings";

export function SeverityTag({ issue }: { issue: HealthIssue }) {
  const ui = useUiTranslations();
  const level = findingLevel(issue);
  return (
    <span
      className={cn(
        "inline-flex shrink-0 items-center px-1.5 py-0.5 text-[11px] font-medium",
        level === "high" && "bg-destructive text-destructive-foreground",
        level === "medium" && "bg-warning/15 text-foreground",
        level === "low" && "bg-muted text-muted-foreground",
      )}
    >
      {level === "high" ? ui("High") : level === "medium" ? ui("Medium") : ui("Low")}
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
  const ui = useUiTranslations();
  const { displayLocale } = useLocale();
  const groups = groupFindings(issues).slice(0, limit);
  if (groups.length === 0) {
    return <p className="px-4 py-6 text-sm text-muted-foreground">{emptyText}</p>;
  }
  return (
    <ul className="divide-y">
      {groups.map(({ lead: issue, similar }) => (
        <li key={issue.id}>
          <Link
            href={`/evals/findings/${issue.id}`}
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
                similar > 0
                  ? similar === 1
                    ? ui("1 similar finding")
                    : ui("{count} similar findings", { count: similar })
                  : null,
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
