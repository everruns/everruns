"use client";

import { useUiTranslations, useLocale } from "@/providers/locale-provider";
import { Shield } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import type { Capability } from "@/lib/api/types";
import { CapabilityIcon } from "@/lib/capability-icons";
import { localizedCapabilityName } from "@/lib/capability-localization";

/** The same read-only capability presentation for saved agents and import candidates. */
export function AgentCapabilityList({
  references,
  capabilities,
}: {
  references: string[];
  capabilities: Capability[];
}) {
  const ui = useUiTranslations();
  const { locale } = useLocale();
  const capabilityById = new Map(capabilities.map((cap) => [cap.id, cap]));
  return references.length ? (
    <ol className="flex flex-wrap gap-1" aria-label={ui("Enabled capabilities")}>
      {references.map((reference) => {
        const cap = capabilityById.get(reference);
        return (
          <li
            key={reference}
            className="inline-flex max-w-full min-w-0 items-center gap-1 border bg-background px-1.5 py-0.5 text-[12px] leading-4"
          >
            {cap && <CapabilityIcon icon={cap.icon} className="size-3 shrink-0" />}
            <span className="truncate">
              {cap ? localizedCapabilityName(cap, locale) : reference}
            </span>
            {cap?.is_guardrail && (
              <Badge variant="outline" className="h-4 gap-0.5 px-1 py-0">
                <Shield />
                {ui("Guardrail")}
              </Badge>
            )}
          </li>
        );
      })}
    </ol>
  ) : (
    <p className="text-[13px] text-muted-foreground">{ui("No capabilities enabled.")}</p>
  );
}
