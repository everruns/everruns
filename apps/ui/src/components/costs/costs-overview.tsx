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
import { getCostSummary, type CostSummary, type CostSummaryLine } from "@/lib/api/costs";
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
