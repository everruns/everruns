/**
 * One line saying what a live session is doing right now, read from its work
 * report: shared by the run card, the thread board and the sessions list. It
 * renders `fallback` until the report arrives, and for rows that are not live.
 */
"use client";

import type { ReactNode } from "react";
import { useLiveWorkReport } from "@/hooks/use-turn-work-report";
import { workReportStatus } from "@/components/chat/work-report-parts";
import { useLocale } from "@/providers/locale-provider";
import { cn } from "@/lib/utils";

export function WorkHeadline({
  sessionId,
  live,
  fallback,
  className,
}: {
  sessionId: string | undefined;
  live: boolean;
  fallback: ReactNode;
  className?: string;
}) {
  const { t } = useLocale();
  const report = useLiveWorkReport(sessionId, live);
  const headline = report ? workReportStatus(report, t) : undefined;
  if (!headline) return <>{fallback}</>;
  const plan = report?.plan;
  return (
    <span
      className={cn("truncate", report?.state === "needs_you" && "text-warning", className)}
      data-testid="work-headline"
      title={headline}
    >
      {plan && plan.total > 0 && report?.state === "working"
        ? `${t("work_plan_step", { current: Math.min(plan.done + 1, plan.total), total: plan.total })} · ${headline}`
        : headline}
    </span>
  );
}
