"use client";

import { useQuery } from "@tanstack/react-query";

import { BASIS_LABELS, BasisBadge, formatQuantity, formatUsd } from "@/components/costs/cost-basis";
import { Notice, NoticeDescription, NoticeTitle } from "@/components/ui/notice";
import { Skeleton } from "@/components/ui/skeleton";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import {
  getCostSummary,
  type CostSummary,
  type CostSummaryLine,
  type RunsWithAgentsLine,
} from "@/lib/api/costs";
import { useUiTranslations } from "@/providers/locale-provider";

function Amount({ line }: { line: CostSummaryLine }) {
  // A line with only unpriced usage has no cost, which is not the same as $0.
  if (line.basis === "unpriced") return <span className="text-muted-foreground">—</span>;
  return <span>{formatUsd(line.cost_usd)}</span>;
}

function Tile({ line }: { line: CostSummaryLine }) {
  const ui = useUiTranslations();
  return (
    <div className="space-y-1 border bg-card px-4 py-3">
      <div className="text-xs text-muted-foreground">{ui(BASIS_LABELS[line.basis])}</div>
      <div className="text-xl font-semibold tabular-nums">
        <Amount line={line} />
      </div>
      <div className="text-xs text-muted-foreground">
        {ui("{count} usage records", { count: String(line.records) })}
      </div>
    </div>
  );
}

function SectionTable({
  title,
  note,
  head,
  rows,
}: {
  title: string;
  note?: string;
  head: string[];
  rows: React.ReactNode[][];
}) {
  const ui = useUiTranslations();
  return (
    <section className="border bg-card">
      <div className="flex items-baseline justify-between gap-3 border-b px-4 py-3">
        <h2 className="text-sm font-semibold">{title}</h2>
        {note ? <span className="text-xs text-muted-foreground">{note}</span> : null}
      </div>
      {rows.length === 0 ? (
        <p className="px-4 py-3 text-sm text-muted-foreground">{ui("No usage in this period.")}</p>
      ) : (
        <Table>
          <TableHeader>
            <TableRow>
              {head.map((h, i) => (
                <TableHead key={h} className={i > 0 ? "text-right" : undefined}>
                  {h}
                </TableHead>
              ))}
            </TableRow>
          </TableHeader>
          <TableBody>
            {rows.map((cells, r) => (
              <TableRow key={r}>
                {cells.map((cell, i) => (
                  <TableCell key={i} className={i > 0 ? "text-right tabular-nums" : undefined}>
                    {cell}
                  </TableCell>
                ))}
              </TableRow>
            ))}
          </TableBody>
        </Table>
      )}
    </section>
  );
}

/**
 * Runs where an agent handed work to other agents: each leading agent with the
 * whole cost of its runs and who spent it. Every amount is also under By
 * agent, for the agent that spent it, so nothing is counted twice; an agent
 * working for two leads shows under each (knowledge/runtime-resources/agents-using-agents.md).
 */
export function RunsWithAgentsCard({
  lines,
  note,
}: {
  lines: RunsWithAgentsLine[];
  note?: string;
}) {
  const ui = useUiTranslations();
  const shared = new Map<string, { name: string; leads: { lead: string; cost: number }[] }>();
  for (const line of lines) {
    for (const share of line.by_agent) {
      if (share.relation === "lead" || !share.agent_id) continue;
      const entry = shared.get(share.agent_id) ?? {
        name: share.agent_name ?? share.agent_id,
        leads: [],
      };
      entry.leads.push({ lead: line.agent_name, cost: share.cost_usd });
      shared.set(share.agent_id, entry);
    }
  }
  const twice = [...shared.values()].filter((entry) => entry.leads.length > 1);
  return (
    <section className="border bg-card">
      <div className="flex items-baseline justify-between gap-3 border-b px-4 py-3">
        <h2 className="text-sm font-semibold">{ui("Runs that use other agents")}</h2>
        {note ? <span className="text-xs text-muted-foreground">{note}</span> : null}
      </div>
      <Table>
        <TableHeader>
          <TableRow>
            <TableHead>{ui("Agent")}</TableHead>
            <TableHead className="text-right">{ui("Cost")}</TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          {lines.flatMap((line) => [
            <TableRow key={line.agent_id}>
              <TableCell>
                <div className="font-medium">{line.agent_name}</div>
                <div className="text-xs text-muted-foreground">
                  {line.runs === 1 ? ui("1 run") : ui("{count} runs", { count: String(line.runs) })}
                </div>
              </TableCell>
              <TableCell className="text-right font-semibold tabular-nums">
                {formatUsd(line.cost_usd)}
              </TableCell>
            </TableRow>,
            ...line.by_agent.map((share) => (
              <TableRow key={`${line.agent_id}:${share.agent_id ?? "none"}`}>
                <TableCell className="pl-8">
                  {share.relation === "lead"
                    ? ui("{name} itself", { name: line.agent_name })
                    : (share.agent_name ?? ui("No agent"))}
                  {share.relation === "created_here" ? (
                    <span className="ml-2 border px-1.5 py-0.5 text-xs text-muted-foreground">
                      {ui("Created here")}
                    </span>
                  ) : share.relation === "linked" && share.agent_id ? (
                    <span className="ml-2 border border-amber-500/40 px-1.5 py-0.5 text-xs text-amber-700 dark:text-amber-300">
                      {ui("Linked")}
                    </span>
                  ) : null}
                </TableCell>
                <TableCell className="text-right tabular-nums">
                  {formatUsd(share.cost_usd)}
                </TableCell>
              </TableRow>
            )),
          ])}
        </TableBody>
      </Table>
      {twice.length > 0 ? (
        <p className="border-t px-4 py-3 text-xs text-muted-foreground">
          {twice
            .map((entry) =>
              ui("{name} worked for {count} agents: {parts}. By agent shows it as one row.", {
                name: entry.name,
                count: String(entry.leads.length),
                parts: entry.leads
                  .map((part) => `${formatUsd(part.cost)} ${ui("for")} ${part.lead}`)
                  .join(", "),
              }),
            )
            .join(" ")}
        </p>
      ) : null}
    </section>
  );
}

function quantityLabel(line: CostSummaryLine): string {
  return line.key?.startsWith("sandbox")
    ? formatQuantity(line.quantity, "hours")
    : formatQuantity(line.quantity, "tokens");
}

export function CostsOverviewView({ summary }: { summary: CostSummary }) {
  const ui = useUiTranslations();
  const month = new Date(summary.from).toLocaleString("en-US", {
    month: "long",
    timeZone: summary.timezone,
  });
  const total: CostSummaryLine = {
    basis: summary.by_basis.length === 1 ? summary.by_basis[0].basis : "mixed",
    cost_usd: summary.cost_usd,
    unpriced_records: summary.unpriced_records,
    records: 0,
    quantity: 0,
  };
  const startedBy = summary.by_started_by.map((line) =>
    line.key === "unattended" ? ui("Unattended work") : ui("People"),
  );
  return (
    <div className="space-y-6">
      {summary.unpriced_records > 0 ? (
        <Notice variant="warning">
          <NoticeTitle>{ui("Some usage this period has no price")}</NoticeTitle>
          <NoticeDescription>
            {ui(
              "{count} usage records have no rate and no provider or catalog price. Totals that include them are partial. Add a rate in the price list to price them.",
              { count: String(summary.unpriced_records) },
            )}
          </NoticeDescription>
        </Notice>
      ) : null}

      <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-4">
        <div className="space-y-1 border bg-card px-4 py-3">
          <div className="text-xs text-muted-foreground">{ui("{month} so far", { month })}</div>
          <div className="text-2xl font-semibold tabular-nums">
            {formatUsd(summary.cost_usd)}
            {summary.unpriced_records > 0 ? (
              <span className="ml-2 text-xs font-normal text-muted-foreground">
                {ui("partial")}
              </span>
            ) : null}
          </div>
          <BasisBadge basis={total.basis} />
        </div>
        {summary.by_basis
          .filter((line) => line.basis !== "unpriced")
          .map((line) => (
            <Tile key={line.basis} line={line} />
          ))}
      </div>

      <div className="grid gap-6 xl:grid-cols-2">
        <SectionTable
          title={ui("By agent")}
          note={month}
          head={[ui("Agent"), ui("Cost"), ui("From")]}
          rows={summary.by_agent.map((line) => [
            line.label ?? ui("No agent"),
            <Amount key="a" line={line} />,
            <BasisBadge key="b" basis={line.basis} />,
          ])}
        />
        <SectionTable
          title={ui("By source")}
          note={month}
          head={[ui("Source"), ui("Usage"), ui("Cost"), ui("From")]}
          rows={summary.by_source.map((line) => [
            line.label ?? line.key ?? "—",
            quantityLabel(line),
            <Amount key="a" line={line} />,
            <BasisBadge key="b" basis={line.basis} />,
          ])}
        />
      </div>

      {summary.runs_with_agents?.length ? (
        <RunsWithAgentsCard lines={summary.runs_with_agents} note={month} />
      ) : null}

      <SectionTable
        title={ui("By who started the work")}
        note={ui("Unattended work is schedules and triggers no person started")}
        head={[ui("Started by"), ui("Usage records"), ui("Cost")]}
        rows={[
          ...summary.by_started_by.map((line, i) => [
            startedBy[i],
            String(line.records),
            <Amount key="a" line={line} />,
          ]),
          ...summary.by_user
            .filter((line) => line.key)
            .map((line) => [
              line.label ?? ui("Unknown person"),
              String(line.records),
              <Amount key="a" line={line} />,
            ]),
        ]}
      />
    </div>
  );
}

export function CostsOverview() {
  const ui = useUiTranslations();
  const query = useQuery({ queryKey: ["costs", "summary"], queryFn: () => getCostSummary() });
  if (query.isLoading) return <Skeleton className="h-64 w-full" />;
  if (query.error || !query.data) return <p role="alert">{ui("Unable to load costs.")}</p>;
  return <CostsOverviewView summary={query.data} />;
}
