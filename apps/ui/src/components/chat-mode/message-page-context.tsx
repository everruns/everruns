"use client";

import { Eye } from "lucide-react";

export function MessagePageContext({ value }: { value: unknown }) {
  if (!value || typeof value !== "object" || !("label" in value) || typeof value.label !== "string")
    return null;
  return (
    <p className="mb-1 flex items-center gap-1 text-[11px] font-normal text-muted-foreground">
      <Eye className="size-3" />
      On {value.label}
    </p>
  );
}

export function MessagePageActions({ value }: { value: unknown }) {
  if (!Array.isArray(value)) return null;
  return (
    <>
      {value.slice(-20).map((action, index) =>
        action && typeof action === "object" && typeof action.label === "string" ? (
          <p key={index} className="text-[11px] text-muted-foreground">
            ↖ {action.label.slice(0, 256)}
          </p>
        ) : null,
      )}
    </>
  );
}
