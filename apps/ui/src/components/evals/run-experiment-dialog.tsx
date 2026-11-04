"use client";

// Start an experiment: one dataset against two to four variants, each a model
// choice over the dataset's own target. The first variant is the baseline.
// Variants that change the harness, agent or prompt stay in the API and CLI
// until agent revisions can be pinned (knowledge/evaluation/experiments.md).

import { useState } from "react";
import { useRouter } from "next/navigation";
import { Plus, X } from "lucide-react";
import { useUiTranslations } from "@/providers/locale-provider";
import { useCreateEvalExperiment } from "@/hooks";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Field } from "@/components/ui/field";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { ModelPicker } from "@/components/models/model-picker";
import type { CreateEvalExperimentRequest, Eval } from "@/lib/api/types";

const MAX_VARIANTS = 4;
const DEFAULT_MODEL = "default";

interface VariantDraft {
  label: string;
  model: string;
}

export interface RunExperimentDraft {
  evalId: string;
  variants: VariantDraft[];
  trials: string;
  maxCost: string;
}

/** Turns the form into a request, or the first thing wrong with it. */
export function experimentRequest(
  draft: RunExperimentDraft,
): { request: CreateEvalExperimentRequest } | { error: string } {
  if (!draft.evalId) return { error: "Choose a dataset." };
  const labels = draft.variants.map((v) => v.label.trim());
  if (labels.some((label) => !label)) return { error: "Every variant needs a label." };
  if (new Set(labels).size !== labels.length) return { error: "Variant labels must differ." };
  const trials = Number(draft.trials);
  if (!Number.isInteger(trials) || trials < 1 || trials > 20) {
    return { error: "Trials are a whole number from 1 to 20." };
  }
  let maxCost: number | undefined;
  if (draft.maxCost.trim()) {
    maxCost = Number(draft.maxCost);
    if (!Number.isFinite(maxCost) || maxCost <= 0) {
      return { error: "The spending cap is a positive amount in USD." };
    }
  }
  return {
    request: {
      variants: draft.variants.map((variant, index) => ({
        label: labels[index],
        ...(variant.model && variant.model !== DEFAULT_MODEL
          ? { model_override: variant.model }
          : {}),
      })),
      trials,
      ...(maxCost !== undefined ? { max_cost_usd: maxCost } : {}),
    },
  };
}

export function RunExperimentDialog({
  open,
  onOpenChange,
  evals,
  defaultEvalId,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** Datasets to choose from: active evals with at least one case. */
  evals: Eval[];
  defaultEvalId?: string;
}) {
  const ui = useUiTranslations();
  const router = useRouter();
  const create = useCreateEvalExperiment();
  const runnable = evals.filter((ev) => (ev.case_count ?? 0) > 0 && ev.target?.type === "session");
  const [draft, setDraft] = useState<RunExperimentDraft>(() => ({
    evalId: defaultEvalId ?? runnable[0]?.id ?? "",
    variants: [
      { label: "baseline", model: DEFAULT_MODEL },
      { label: "candidate", model: DEFAULT_MODEL },
    ],
    trials: "3",
    maxCost: "",
  }));
  const [error, setError] = useState<string | null>(null);

  const setVariant = (index: number, patch: Partial<VariantDraft>) =>
    setDraft((d) => ({
      ...d,
      variants: d.variants.map((v, i) => (i === index ? { ...v, ...patch } : v)),
    }));

  const submit = async (event: React.FormEvent) => {
    event.preventDefault();
    const built = experimentRequest(draft);
    if ("error" in built) {
      setError(ui(built.error));
      return;
    }
    setError(null);
    try {
      const experiment = await create.mutateAsync({
        evalId: draft.evalId,
        request: built.request,
      });
      onOpenChange(false);
      router.push(`/evals/${experiment.eval_id}/experiments/${experiment.id}`);
    } catch (err) {
      setError(err instanceof Error ? err.message : ui("Could not start the experiment."));
    }
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent
        chatProtected="Run experiment"
        className="max-h-[calc(100svh-2rem)] max-w-xl overflow-y-auto"
      >
        <DialogHeader>
          <DialogTitle>{ui("Run experiment")}</DialogTitle>
          <DialogDescription>
            {ui(
              "Run one dataset against several variants and compare them case by case. The first variant is the baseline.",
            )}
          </DialogDescription>
        </DialogHeader>
        <form onSubmit={submit} className="space-y-4">
          <Field id="experiment-dataset" label={ui("Dataset")}>
            <Select
              value={draft.evalId}
              onValueChange={(value) => setDraft((d) => ({ ...d, evalId: value }))}
            >
              <SelectTrigger id="experiment-dataset">
                <SelectValue placeholder={ui("Choose a dataset")} />
              </SelectTrigger>
              <SelectContent>
                {runnable.map((ev) => (
                  <SelectItem key={ev.id} value={ev.id}>
                    {ev.name} · {ui("{count} cases", { count: ev.case_count ?? 0 })}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </Field>

          <fieldset className="space-y-2">
            <legend className="text-sm font-medium">{ui("Variants")}</legend>
            {draft.variants.map((variant, index) => (
              <div key={index} className="grid grid-cols-[8rem_minmax(0,1fr)_auto] gap-2">
                <Input
                  aria-label={ui("Variant {n} label", { n: index + 1 })}
                  value={variant.label}
                  onChange={(e) => setVariant(index, { label: e.target.value })}
                />
                <ModelPicker
                  id={`experiment-variant-${index}-model`}
                  value={variant.model}
                  onChange={(model) => setVariant(index, { model })}
                  additionalOptions={[{ value: DEFAULT_MODEL, label: ui("Dataset's own model") }]}
                />
                <Button
                  type="button"
                  variant="ghost"
                  size="icon"
                  aria-label={ui("Remove variant {n}", { n: index + 1 })}
                  disabled={draft.variants.length <= 2}
                  onClick={() =>
                    setDraft((d) => ({
                      ...d,
                      variants: d.variants.filter((_, i) => i !== index),
                    }))
                  }
                >
                  <X className="size-4" />
                </Button>
              </div>
            ))}
            {draft.variants.length < MAX_VARIANTS && (
              <Button
                type="button"
                variant="outline"
                size="sm"
                onClick={() =>
                  setDraft((d) => ({
                    ...d,
                    variants: [
                      ...d.variants,
                      {
                        label: `variant-${d.variants.length + 1}`,
                        model: DEFAULT_MODEL,
                      },
                    ],
                  }))
                }
              >
                <Plus className="size-4" />
                {ui("Add variant")}
              </Button>
            )}
          </fieldset>

          <div className="grid grid-cols-2 gap-3">
            <Field
              id="experiment-trials"
              label={ui("Trials per case")}
              description={ui("More trials separate noise from real changes.")}
            >
              <Input
                id="experiment-trials"
                inputMode="numeric"
                value={draft.trials}
                onChange={(e) => setDraft((d) => ({ ...d, trials: e.target.value }))}
                aria-describedby="experiment-trials-hint"
              />
            </Field>
            <div className="space-y-1.5">
              <Label htmlFor="experiment-cap">
                {ui("Spending cap, USD")}{" "}
                <span className="font-normal text-muted-foreground">{ui("(optional)")}</span>
              </Label>
              <Input
                id="experiment-cap"
                inputMode="decimal"
                placeholder="10"
                value={draft.maxCost}
                onChange={(e) => setDraft((d) => ({ ...d, maxCost: e.target.value }))}
              />
              <p className="text-xs text-muted-foreground">
                {ui("Split evenly between the variants.")}
              </p>
            </div>
          </div>

          {error && (
            <p role="alert" className="text-sm text-destructive">
              {error}
            </p>
          )}
          <DialogFooter>
            <Button type="submit" disabled={create.isPending || runnable.length === 0}>
              {create.isPending ? ui("Starting…") : ui("Run experiment")}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
