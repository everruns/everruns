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
import { getDisplayName } from "@/lib/entity-lifecycle";

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
  /** Include a "no harness" option */
  includeNoneOption?: boolean;
  /** Label for the empty option */
  noneLabel?: string;
  /** Harness IDs to omit from the dropdown */
  excludeIds?: string[];
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
  includeNoneOption = false,
  noneLabel = "None",
  excludeIds = [],
}: HarnessSelectProps) {
  const noneValue = "__none__";
  const { data: harnesses = [] } = useHarnesses();
  const filteredHarnesses = useMemo(
    () => harnesses.filter((harness) => !excludeIds.includes(harness.id)),
    [excludeIds, harnesses],
  );

  const harnessMap = useMemo(() => {
    return new Map<string, Harness>(filteredHarnesses.map((h) => [h.id, h]));
  }, [filteredHarnesses]);

  // Resolve ID → name. When harnesses haven't loaded yet (org switch race),
  // show "Loading…" instead of the raw ID (EVE-142).
  // When no value is set and includeNoneOption is true, show the noneLabel.
  const displayValue = value
    ? getDisplayName(harnessMap.get(value)) || (filteredHarnesses.length === 0 ? "Loading…" : value)
    : includeNoneOption
      ? noneLabel
      : undefined;

  return (
    <Select
      value={value || (includeNoneOption ? noneValue : value)}
      onValueChange={(nextValue) => onValueChange(nextValue === noneValue ? "" : nextValue)}
      disabled={disabled}
    >
      <SelectTrigger className={className}>
        <SelectValue placeholder={placeholder}>{displayValue}</SelectValue>
      </SelectTrigger>
      <SelectContent>
        {includeNoneOption && <SelectItem value={noneValue}>{noneLabel}</SelectItem>}
        {filteredHarnesses.map((harness) => (
          <SelectItem key={harness.id} value={harness.id}>
            {getDisplayName(harness)}
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
  );
}
