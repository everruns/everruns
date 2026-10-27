"use client";

import { localizedEntityDescription } from "@/lib/entity-localization";
import { useLocale as useDisplayLocale } from "@/providers/locale-provider";
import { useUiTranslations } from "@/providers/locale-provider";
import { useId, useMemo, useState } from "react";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { useHarnesses } from "@/hooks";
import type { Harness } from "@/lib/api/types";
import { isHarnessDeprecated } from "@/lib/harness-deprecation";
import { getDisplayName } from "@/lib/entity-lifecycle";

// The decision guide lives in the public docs so the picker and the docs never
// disagree about when each harness fits.
export const HARNESS_GUIDE_URL = "https://everruns.com/docs/features/harnesses/#choosing-a-harness";

interface HarnessSelectProps {
  /** Optional ID for the select trigger */
  id?: string;
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
 * Each option, and the closed control, includes the harness purpose so the
 * choice is understandable without opening the harness itself.
 */
export function HarnessSelect({
  id,
  value,
  onValueChange,
  placeholder = "Select harness",
  className,
  disabled,
  includeNoneOption = false,
  noneLabel = "None",
  excludeIds = [],
}: HarnessSelectProps) {
  const { displayLocale } = useDisplayLocale();
  const ui = useUiTranslations();
  const [showDeprecated, setShowDeprecated] = useState(false);
  const noneValue = "__none__";
  const purposeId = `${useId()}-purpose`;
  const { data: harnesses = [] } = useHarnesses();
  const filteredHarnesses = useMemo(
    () =>
      harnesses.filter(
        (harness) =>
          !excludeIds.includes(harness.id) &&
          (showDeprecated || harness.id === value || !isHarnessDeprecated(harness)),
      ),
    [excludeIds, harnesses, showDeprecated, value],
  );

  const harnessMap = useMemo(() => {
    return new Map<string, Harness>(filteredHarnesses.map((h) => [h.id, h]));
  }, [filteredHarnesses]);

  // Resolve ID → name. When harnesses haven't loaded yet (org switch race),
  // show "Loading…" instead of the raw ID (EVE-142).
  // When no value is set and includeNoneOption is true, show the noneLabel.
  const selected = value ? harnessMap.get(value) : undefined;
  const displayValue = value
    ? getDisplayName(selected, displayLocale) ||
      (filteredHarnesses.length === 0 ? ui("Loading…") : value)
    : includeNoneOption
      ? ui(noneLabel)
      : undefined;
  const purpose = selected ? localizedEntityDescription(selected, displayLocale).trim() : "";

  return (
    <div className="flex w-full min-w-0 flex-col gap-1.5">
      <Select
        value={value || (includeNoneOption ? noneValue : value)}
        onValueChange={(nextValue) => onValueChange(nextValue === noneValue ? "" : nextValue)}
        disabled={disabled}
      >
        <SelectTrigger
          id={id}
          className={className}
          aria-describedby={purpose ? purposeId : undefined}
        >
          <SelectValue placeholder={ui(placeholder)}>{displayValue}</SelectValue>
        </SelectTrigger>
        <SelectContent className="w-72">
          {includeNoneOption && <SelectItem value={noneValue}>{ui(noneLabel)}</SelectItem>}
          {harnesses.some(isHarnessDeprecated) && (
            <button
              type="button"
              className="w-full px-2 py-1.5 text-left text-sm text-muted-foreground"
              onClick={() => setShowDeprecated((shown) => !shown)}
              aria-pressed={showDeprecated}
            >
              {showDeprecated ? ui("Hide deprecated") : ui("Show deprecated")}
            </button>
          )}
          {filteredHarnesses.map((harness) => {
            const description = localizedEntityDescription(harness, displayLocale).trim();
            return (
              <SelectItem
                key={harness.id}
                value={harness.id}
                className="items-start py-1.5 [&>div]:min-w-0 [&>div]:flex-1"
              >
                <span className="flex w-full min-w-0 flex-col items-start gap-0.5 whitespace-normal">
                  <span>{getDisplayName(harness, displayLocale)}</span>
                  {description ? (
                    <span className="line-clamp-2 text-left text-[11px] leading-snug font-normal text-muted-foreground">
                      {description}
                    </span>
                  ) : null}
                </span>
              </SelectItem>
            );
          })}
        </SelectContent>
      </Select>
      {purpose ? (
        <p id={purposeId} className="line-clamp-3 text-xs leading-snug text-muted-foreground">
          {purpose}
        </p>
      ) : null}
      <a
        href={HARNESS_GUIDE_URL}
        target="_blank"
        rel="noreferrer"
        className="w-fit text-xs text-muted-foreground underline underline-offset-2 hover:text-foreground"
      >
        {ui("Which harness should I use?")}
      </a>
    </div>
  );
}
