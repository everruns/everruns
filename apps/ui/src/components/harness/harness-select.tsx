"use client";

import { useMemo } from "react";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { useHarnesses } from "@/hooks";
import type { Harness } from "@/lib/api/types";

interface HarnessSelectProps {
  /** Selected harness ID */
  value: string;
  /** Callback when selection changes */
  onValueChange: (harnessId: string) => void;
  /** Placeholder text when nothing selected */
  placeholder?: string;
  /** Additional className for trigger */
  className?: string;
  /** Disable the select */
  disabled?: boolean;
}

/**
 * Harness select dropdown that displays harness names but uses IDs as values.
 * Handles the ID-to-name mapping automatically.
 */
export function HarnessSelect({
  value,
  onValueChange,
  placeholder = "Select harness",
  className,
  disabled,
}: HarnessSelectProps) {
  const { data: harnesses = [] } = useHarnesses();

  const harnessMap = useMemo(() => {
    return new Map<string, Harness>(harnesses.map((h) => [h.id, h]));
  }, [harnesses]);

  // Resolve ID → name. When harnesses haven't loaded yet (org switch race),
  // show "Loading…" instead of the raw ID (EVE-142).
  const displayValue = value
    ? (harnessMap.get(value)?.name ?? (harnesses.length === 0 ? "Loading…" : value))
    : undefined;

  return (
    <Select value={value} onValueChange={onValueChange} disabled={disabled}>
      <SelectTrigger className={className}>
        <SelectValue placeholder={placeholder}>{displayValue}</SelectValue>
      </SelectTrigger>
      <SelectContent>
        {harnesses.map((harness) => (
          <SelectItem key={harness.id} value={harness.id}>
            {harness.name}
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
  );
}
