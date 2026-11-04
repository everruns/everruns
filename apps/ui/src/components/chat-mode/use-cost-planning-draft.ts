"use client";

import { useState } from "react";
import { z } from "zod";
import { useUiTranslations } from "@/providers/locale-provider";
import { usePageFormDraft } from "./use-page-form-draft";

export const planningNumberInput = z
  .string()
  .max(40)
  .refine((value) => value === "" || (value.trim() !== "" && Number.isFinite(Number(value))));

// Local calculators outrank the Agent underneath their drawer. They have no
// entity ID, persistence callback, billing authority or saved-history inverse.
export function useCostPlanningDraft<T extends Record<string, unknown>>(
  formId: string,
  schema: z.ZodType<T>,
  values: T,
  apply: (values: T) => void,
  enabled = true,
) {
  const ui = useUiTranslations();
  const serialized = JSON.stringify(values);
  const [initial] = useState(serialized);
  return usePageFormDraft({
    formId,
    schema,
    values,
    apply,
    entityType: "agent_cost_estimate",
    label: ui("Estimate cost"),
    priority: 50,
    enabled,
    baseline: "local",
    dirty: serialized !== initial,
    writable: true,
    reviewHint: ui("Changes affect only this estimate."),
  });
}
