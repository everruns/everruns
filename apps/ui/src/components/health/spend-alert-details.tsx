"use client";

import Link from "next/link";
import type { HealthIssue } from "@/lib/api/types";
import { LinkButton } from "@/components/ui/button";
import { useLocale, useUiTranslations } from "@/providers/locale-provider";

export function SpendAlertDetails({ issue }: { issue: HealthIssue }) {
  const ui = useUiTranslations();
  const { displayLocale } = useLocale();
  const evidence = issue.evidence ?? {};
  const current = finite(evidence.last_day_usd);
  const baseline = finite(evidence.baseline_daily_usd);
  const money = (value: number) =>
    new Intl.NumberFormat(displayLocale, { style: "currency", currency: "USD" }).format(value);
  const end = typeof evidence.window_end === "string" ? Date.parse(evidence.window_end) : NaN;
  const sessions = Array.isArray(evidence.top_sessions) ? evidence.top_sessions.slice(0, 5) : [];
  return (
    <section aria-label={ui("Spend breakdown")} className="space-y-4 border-t pt-4">
      <h4 className="text-sm font-medium">{ui("Spend breakdown")}</h4>
      {current !== null && baseline !== null && (
        <dl className="grid gap-4 sm:grid-cols-3 text-sm">
          <div>
            <dt className="text-muted-foreground">{ui("Last 24 hours")}</dt>
            <dd className="mt-1 text-lg font-medium">{money(current)}</dd>
          </div>
          <div>
            <dt className="text-muted-foreground">{ui("Previous daily average")}</dt>
            <dd className="mt-1 text-lg font-medium">{money(baseline)}</dd>
          </div>
          <div>
            <dt className="text-muted-foreground">{ui("Increase")}</dt>
            <dd className="mt-1 text-lg font-medium">+{money(Math.max(0, current - baseline))}</dd>
          </div>
        </dl>
      )}
      <p className="text-xs text-muted-foreground">
        {ui(
          "Compares recorded unattended usage over 24 hours with the daily average over the six days before, including days with no usage. Alerts above $5 and 3× that average.",
        )}
      </p>
      {current !== null && baseline !== null && baseline > 0 && (
        <p className="text-sm">
          {ui("{value0}× the daily average", {
            value0: (current / baseline).toLocaleString(displayLocale, {
              maximumFractionDigits: 1,
            }),
          })}
        </p>
      )}
      {Number.isFinite(end) && (
        <p className="text-xs text-muted-foreground">
          {ui("Usage window")}: {new Date(end - 86_400_000).toLocaleString(displayLocale)} –{" "}
          {new Date(end).toLocaleString(displayLocale)}
        </p>
      )}
      <p className="text-sm">
        {ui(
          "An increase can be expected work; it does not mean the agent failed. Review the sessions and triggers before changing its budget.",
        )}
      </p>
      {sessions.length > 0 && (
        <div className="space-y-2">
          <h4 className="text-sm font-medium">{ui("Top sessions by unattended spend")}</h4>
          <ul className="divide-y text-sm">
            {sessions.map((value) => {
              if (!value || typeof value !== "object") return null;
              const { session_id: id, amount_usd: amount } = value as Record<string, unknown>;
              // Evidence is untrusted: only typed local session IDs become links.
              if (
                typeof id !== "string" ||
                !/^session_[0-9a-f]{32}$/.test(id) ||
                finite(amount) === null
              )
                return null;
              return (
                <li key={id} className="flex flex-wrap items-center justify-between gap-2 py-2">
                  <Link className="underline underline-offset-4" href={`/sessions/${id}/cost`}>
                    {ui("Session {value0}", { value0: id.slice(-8) })}
                  </Link>
                  <span>{money(amount as number)}</span>
                  <Link
                    className="text-xs underline underline-offset-4"
                    href={`/sessions/${id}/trace`}
                  >
                    {ui("Trace")}
                  </Link>
                </li>
              );
            })}
          </ul>
        </div>
      )}
      {sessions.length === 0 && (
        <p className="text-xs text-muted-foreground">
          {ui("Session breakdown is unavailable. Check again to refresh this alert.")}
        </p>
      )}
      <div className="flex flex-wrap gap-2">
        {issue.agent_id && (
          <>
            <LinkButton variant="outline" size="sm" href={`/agents/${issue.agent_id}?tab=sessions`}>
              {ui("Agent sessions")}
            </LinkButton>
            <LinkButton
              variant="outline"
              size="sm"
              href={`/agents/${issue.agent_id}?tab=integrations`}
            >
              {ui("Channels and triggers")}
            </LinkButton>
          </>
        )}
        <LinkButton variant="outline" size="sm" href="/settings/costs">
          {ui("Usage policies")}
        </LinkButton>
      </div>
    </section>
  );
}

function finite(value: unknown): number | null {
  return typeof value === "number" && Number.isFinite(value) && value >= 0 ? value : null;
}
