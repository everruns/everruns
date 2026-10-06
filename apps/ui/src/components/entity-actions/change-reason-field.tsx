"use client";

// The single-line "Reason for this change" field in save, delete and restore
// flows. People may leave it empty (the field never blocks a save); a restore
// passes `required`. The text travels as the change-reason header and lands in
// the entity's history. See knowledge/execution/change-reasons-and-manager-context.md.

import { useId } from "react";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { MAX_CHANGE_REASON_CHARS } from "@/lib/api/change-history";
import { cn } from "@/lib/utils";

interface ChangeReasonFieldProps {
  value: string;
  onChange: (value: string) => void;
  required?: boolean;
  disabled?: boolean;
  placeholder?: string;
  className?: string;
}

export function ChangeReasonField({
  value,
  onChange,
  required = false,
  disabled,
  placeholder,
  className,
}: ChangeReasonFieldProps) {
  const id = useId();
  return (
    <div className={cn("space-y-1.5", className)}>
      <Label htmlFor={id} className="text-xs">
        Reason for this change
        {!required && <span className="font-normal text-muted-foreground">(optional)</span>}
      </Label>
      <Input
        id={id}
        value={value}
        onChange={(event) => onChange(event.target.value)}
        maxLength={MAX_CHANGE_REASON_CHARS}
        required={required}
        disabled={disabled}
        placeholder={placeholder ?? "Why you are making this change"}
      />
    </div>
  );
}
