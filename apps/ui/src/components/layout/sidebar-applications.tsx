"use client";
import { ArrowUpRight, PanelsTopLeft } from "lucide-react";
import { useApplicationLaunchers } from "@/hooks/use-applications";
import { useUiTranslations } from "@/providers/locale-provider";
export function SidebarApplications() {
  const { data } = useApplicationLaunchers();
  const ui = useUiTranslations();
  if (!data?.length) return null;
  return (
    <nav aria-label={ui("Published applications")} className="border-t border-border px-3 py-3">
      <div className="mb-2 text-xs text-muted-foreground">{ui("Published applications")}</div>
      {data.map((a) => (
        <a
          key={a.id}
          href={a.url}
          target="_blank"
          rel="noreferrer"
          className="flex items-center gap-2 rounded-md px-2 py-2 text-sm text-foreground hover:bg-muted"
        >
          <PanelsTopLeft className="size-4 shrink-0" />
          <span className="truncate">{a.name}</span>
          <ArrowUpRight className="ml-auto size-3 shrink-0 text-muted-foreground" />
        </a>
      ))}
    </nav>
  );
}
