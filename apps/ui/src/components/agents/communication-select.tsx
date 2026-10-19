"use client";

import { useId } from "react";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import type { Communication } from "@/lib/api/types";
import { COMMUNICATION_OPTIONS, getCommunicationOption } from "@/lib/agent-communication";
import { cn } from "@/lib/utils";

interface CommunicationSelectProps {
  id?: string;
  value: Communication;
  onValueChange: (value: Communication) => void;
  className?: string;
  /** Show the selected option's description under the control. */
  showDescription?: boolean;
}

/** Select for the agent-level Communication setting (`direct` | `explicit`). */
export function CommunicationSelect({
  id,
  value,
  onValueChange,
  className,
  showDescription = true,
}: CommunicationSelectProps) {
  const descriptionId = `${useId()}-communication-description`;
  const selected = getCommunicationOption(value);
  return (
    <>
      <Select value={value} onValueChange={(next) => onValueChange(next as Communication)}>
        <SelectTrigger
          id={id}
          className={cn("w-full", className)}
          aria-describedby={showDescription ? descriptionId : undefined}
        >
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          {COMMUNICATION_OPTIONS.map((option) => (
            <SelectItem key={option.value} value={option.value}>
              {option.label}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
      {showDescription && (
        <p id={descriptionId} className="text-xs leading-relaxed text-muted-foreground">
          {selected.description}
        </p>
      )}
    </>
  );
}
