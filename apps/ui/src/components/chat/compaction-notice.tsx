"use client";
import { useUiTranslations } from "@/providers/locale-provider";

export function CompactionNotice({ failed = false }: { failed?: boolean }) {
  const ui = useUiTranslations();
  return (
    <div
      role={failed ? "alert" : "status"}
      aria-live="polite"
      className={`border-l-2 px-3 py-2 text-xs ${failed ? "border-destructive text-destructive" : "border-border text-muted-foreground"}`}
    >
      {ui(failed ? "Compaction failed. Previous history was retained." : "Compacting context…")}
    </div>
  );
}
