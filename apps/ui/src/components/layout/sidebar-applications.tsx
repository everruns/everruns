"use client";
import { ArrowUpRight, PanelsTopLeft } from "lucide-react";
import { useApplicationLaunchers } from "@/hooks/use-applications";
import { useUiTranslations } from "@/providers/locale-provider";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
export function SidebarApplications({ compact = false }: { compact?: boolean }) {
  const { data } = useApplicationLaunchers();
  const ui = useUiTranslations();
  if (!data?.length) return null;
  return (
    <nav
      aria-label={ui("Published applications")}
      className={compact ? "border-t border-border py-1.5" : "border-t border-border px-3 py-3"}
    >
      {!compact && (
        <div className="mb-2 text-xs text-muted-foreground">{ui("Published applications")}</div>
      )}
      {data.map((a) => {
        const link = (
          <a
            key={a.id}
            href={a.url}
            target="_blank"
            rel="noreferrer"
            aria-label={compact ? a.name : undefined}
            className={
              compact
                ? "flex h-10 w-12 shrink-0 items-center justify-center text-muted-foreground hover:bg-muted hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring"
                : "flex items-center gap-2 rounded-md px-2 py-2 text-sm text-foreground hover:bg-muted"
            }
          >
            <PanelsTopLeft className="size-4 shrink-0" />
            {!compact && (
              <>
                <span className="truncate">{a.name}</span>
                <ArrowUpRight className="ml-auto size-3 shrink-0 text-muted-foreground" />
              </>
            )}
          </a>
        );
        return compact ? (
          <Tooltip key={a.id}>
            <TooltipTrigger asChild delay={150}>
              {link}
            </TooltipTrigger>
            <TooltipContent side="left" sideOffset={8}>
              {a.name}
            </TooltipContent>
          </Tooltip>
        ) : (
          link
        );
      })}
    </nav>
  );
}
