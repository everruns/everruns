"use client";
import type { ReactNode } from "react";
import { ToggleGroup } from "@base-ui/react/toggle-group";
import { Toggle } from "@base-ui/react/toggle";
import { cn } from "@/lib/utils";

export function SegmentedControl({
  label,
  value,
  onValueChange,
  options,
  className,
}: {
  label: string;
  value: string;
  onValueChange: (value: string) => void;
  options: { value: string; label: string; icon?: ReactNode }[];
  className?: string;
}) {
  return (
    <ToggleGroup
      aria-label={label}
      value={[value]}
      onValueChange={(values) => {
        if (values[0]) onValueChange(values[0]);
      }}
      className={cn("inline-flex max-w-full flex-wrap border", className)}
    >
      {options.map((o) => (
        <Toggle
          key={o.value}
          value={o.value}
          aria-label={o.label}
          className="flex h-8 items-center gap-2 px-3 text-[13px] text-muted-foreground hover:bg-muted focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring data-[pressed]:bg-muted data-[pressed]:text-foreground"
        >
          {o.icon}
          {o.label}
        </Toggle>
      ))}
    </ToggleGroup>
  );
}
