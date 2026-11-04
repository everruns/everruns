"use client";
import { useId, useMemo, useState } from "react";
import { z } from "zod";
import {
  planningNumberInput,
  useCostPlanningDraft,
} from "@/components/chat-mode/use-cost-planning-draft";
import { useChatPageRows } from "@/components/chat-mode/page-view-state";
import { useLocale, useUiTranslations } from "@/providers/locale-provider";
import { formatPlanningUsd } from "@/lib/agent-cost-projection";
import type { CostRate, OperatingCostLine } from "@/lib/agent-operating-cost";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";

const EXTERNAL = [
  { id: "sandbox", label: "Sandbox runtime", unit: "minute" },
  { id: "tools", label: "Paid tools", unit: "call" },
  { id: "media", label: "Media generation", unit: "item" },
  { id: "subagents", label: "Subagent work", unit: "task" },
  { id: "hosting", label: "Hosting and platform fees", unit: "month" },
];

export function OperatingCostEditor({
  lines,
  rates,
  onChange,
  dailyTasks,
}: {
  lines: OperatingCostLine[];
  rates: CostRate[];
  onChange: (lines: OperatingCostLine[]) => void;
  dailyTasks: number | null;
}) {
  const ui = useUiTranslations();
  const { displayLocale } = useLocale();
  const id = useId();
  const [selection, setSelection] = useState("external:sandbox");
  const [sequence, setSequence] = useState(0);
  const shapeKey = JSON.stringify({
    lines: lines.map(({ id, rateId, label, unit, period }) => ({
      id,
      rateId,
      label,
      unit,
      period,
    })),
    rateIds: rates.map(({ id }) => id),
  });
  // Schema identity changes with row structure, not with edited quantities.
  const schema = useMemo(() => {
    const shape: {
      lines: Pick<OperatingCostLine, "id" | "rateId" | "label" | "unit" | "period">[];
      rateIds: string[];
    } = JSON.parse(shapeKey);
    return z.strictObject({
      selection: z.enum([
        ...EXTERNAL.map((item) => "external:" + item.id),
        ...shape.rateIds.map((id) => "rate:" + id),
      ] as [string, ...string[]]),
      costs: z.strictObject(
        Object.fromEntries(
          shape.lines.map((line) => [
            line.id,
            z
              .strictObject({
                quantity: planningNumberInput,
                ...(!line.rateId ? { usdPerUnit: planningNumberInput } : {}),
              })
              .describe(line.label.slice(0, 200) + " (" + line.unit + " per " + line.period + ")"),
          ]),
        ),
      ),
    });
  }, [shapeKey]);
  const values = {
    selection,
    costs: Object.fromEntries(
      lines.map((line) => [
        line.id,
        {
          quantity: Number.isNaN(line.quantity) ? "" : String(line.quantity),
          ...(!line.rateId
            ? { usdPerUnit: line.usdPerUnit === undefined ? "" : String(line.usdPerUnit) }
            : {}),
        },
      ]),
    ),
  };
  const draft = useCostPlanningDraft("operating_costs", schema, values, (next) => {
    setSelection(next.selection);
    onChange(
      lines.map((line) => ({
        ...line,
        quantity: next.costs[line.id].quantity === "" ? NaN : Number(next.costs[line.id].quantity),
        ...(!line.rateId
          ? {
              usdPerUnit:
                next.costs[line.id].usdPerUnit === ""
                  ? undefined
                  : Number(next.costs[line.id].usdPerUnit),
            }
          : {}),
      })),
    );
  });
  useChatPageRows(
    "Operating cost assumptions",
    lines.map((line) => ({
      id: line.id,
      label: line.label,
      unit: line.unit,
      period: line.period,
      billing: line.billing,
      usd_per_unit: line.usdPerUnit,
    })),
  );
  const update = (index: number, patch: Partial<OperatingCostLine>) =>
    onChange(lines.map((line, i) => (i === index ? { ...line, ...patch } : line)));
  return (
    <section aria-label={ui("Other operating costs")} className="space-y-4 border p-5">
      {draft.notice}
      <div>
        <h3 className="text-sm font-semibold">{ui("Other operating costs")}</h3>
        <p className="mt-1 text-xs text-muted-foreground">
          {ui(
            "Include resources used by an average task. External providers bill separately from prepaid credits.",
          )}
        </p>
      </div>
      <div className="flex flex-wrap gap-2">
        <select
          aria-label={ui("Cost category")}
          className="min-w-0 flex-1 border bg-background p-2 text-sm"
          value={selection}
          onChange={(e) => setSelection(e.target.value)}
        >
          {EXTERNAL.map((item) => (
            <option key={item.id} value={`external:${item.id}`}>
              {ui(item.label)}
            </option>
          ))}
          {rates.map((rate) => (
            <option key={rate.id} value={`rate:${rate.id}`}>
              {rate.label}
            </option>
          ))}
        </select>
        <Button
          variant="outline"
          disabled={lines.length >= 20}
          onClick={() => {
            const rate = selection.startsWith("rate:")
              ? rates.find((r) => `rate:${r.id}` === selection)
              : undefined;
            const template = rate ?? EXTERNAL.find((r) => `external:${r.id}` === selection);
            if (!template) return;
            onChange([
              ...lines,
              {
                id: `${id}-${sequence}`,
                label: template.label,
                unit: template.unit,
                quantity: 1,
                period: rate?.period ?? (template.unit === "month" ? "month" : "task"),
                billing: rate?.billing ?? "external",
                rateId: rate?.id,
                usdPerUnit: rate?.usdPerUnit,
              },
            ]);
            setSequence(sequence + 1);
          }}
        >
          {ui("Add cost")}
        </Button>
      </div>
      {lines.map((line, index) => (
        <div
          key={line.id}
          role="group"
          aria-label={ui(line.label)}
          className="space-y-3 border-t pt-4"
        >
          <div className="flex items-center justify-between gap-3">
            <div>
              <p className="text-sm font-medium">{line.rateId ? line.label : ui(line.label)}</p>
              <p className="text-xs text-muted-foreground">
                {ui(line.billing === "credits" ? "Paid from credits" : "Billed externally")}
              </p>
            </div>
            <Button
              size="sm"
              variant="ghost"
              onClick={() => onChange(lines.filter((_, i) => i !== index))}
            >
              {ui("Remove")}
            </Button>
          </div>
          <div className="grid gap-4 sm:grid-cols-2">
            <div className="space-y-2">
              <Label htmlFor={`${line.id}-quantity`}>
                {ui(line.period === "month" ? "Quantity per month" : "Quantity per task")} (
                {ui(line.unit)})
              </Label>
              <Input
                id={`${line.id}-quantity`}
                type="number"
                min={0}
                max={1000000000}
                step="any"
                value={Number.isNaN(line.quantity) ? "" : line.quantity}
                onChange={(e) =>
                  update(index, { quantity: e.target.value === "" ? NaN : Number(e.target.value) })
                }
              />
            </div>
            <div className="space-y-2">
              <Label htmlFor={`${line.id}-price`}>
                {ui("USD per {unit}", { unit: ui(line.unit) })}
              </Label>
              <Input
                id={`${line.id}-price`}
                type="number"
                min={0}
                max={1000000}
                step="any"
                placeholder={ui("Enter provider price")}
                value={line.usdPerUnit ?? ""}
                readOnly={!!line.rateId}
                onChange={(e) =>
                  update(index, {
                    usdPerUnit: e.target.value === "" ? undefined : Number(e.target.value),
                  })
                }
              />
            </div>
          </div>
          <p className="text-xs text-muted-foreground">
            {ui("Monthly estimate: {amount}", {
              amount:
                line.usdPerUnit != null &&
                Number.isFinite(line.quantity) &&
                line.quantity >= 0 &&
                dailyTasks != null
                  ? formatPlanningUsd(
                      line.quantity *
                        line.usdPerUnit *
                        (line.period === "month" ? 1 : dailyTasks * 30),
                      displayLocale,
                    )
                  : "—",
            })}
          </p>
        </div>
      ))}
      <p className="text-xs text-muted-foreground">
        {ui(
          "Use the full sandbox instance price, including memory. For subagents, enter the cost of one child task. Include idle time and repeated tool work in quantities; monthly fees are added once.",
        )}
      </p>
    </section>
  );
}
