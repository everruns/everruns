"use client";

// Experiments newest first: what ran, against which dataset, and where it is.

import Link from "next/link";
import { useLocale, useUiTranslations } from "@/providers/locale-provider";
import { Badge } from "@/components/ui/badge";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { formatRelativeTime } from "@/lib/formatting";
import type { Eval, EvalExperiment } from "@/lib/api/types";

export function ExperimentsTable({
  experiments,
  evals,
  emptyText,
}: {
  experiments: EvalExperiment[];
  evals: Eval[];
  emptyText: string;
}) {
  const ui = useUiTranslations();
  const { displayLocale } = useLocale();
  const names = new Map(evals.map((ev) => [ev.id, ev.name]));
  if (experiments.length === 0) {
    return <p className="px-4 py-6 text-sm text-muted-foreground">{emptyText}</p>;
  }
  return (
    <div className="overflow-x-auto">
      <Table>
        <TableHeader>
          <TableRow>
            <TableHead>{ui("Experiment")}</TableHead>
            <TableHead>{ui("Dataset")}</TableHead>
            <TableHead>{ui("Variants")}</TableHead>
            <TableHead>{ui("Trials")}</TableHead>
            <TableHead>{ui("Status")}</TableHead>
            <TableHead>{ui("Started")}</TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          {experiments.map((experiment) => (
            <TableRow key={experiment.id}>
              <TableCell>
                <Link
                  href={`/evals/${experiment.eval_id}/experiments/${experiment.id}`}
                  className="font-medium hover:underline"
                >
                  {experiment.name}
                </Link>
              </TableCell>
              <TableCell className="text-muted-foreground">
                {names.get(experiment.eval_id) ?? experiment.eval_id}{" "}
                <span className="text-xs">v{experiment.dataset_version}</span>
              </TableCell>
              <TableCell className="tabular-nums">{experiment.variants.length}</TableCell>
              <TableCell className="tabular-nums">{experiment.trials}</TableCell>
              <TableCell>
                <Badge variant={experiment.status === "completed" ? "default" : "outline"}>
                  {experiment.status}
                </Badge>
              </TableCell>
              <TableCell className="text-muted-foreground">
                {formatRelativeTime(experiment.created_at, displayLocale)}
              </TableCell>
            </TableRow>
          ))}
        </TableBody>
      </Table>
    </div>
  );
}
