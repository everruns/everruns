"use client";

import { Shield } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import type { Capability } from "@/lib/api/types";
import { CapabilityIcon } from "@/lib/capability-icons";
import { localizedCapabilityName } from "@/lib/capability-localization";
import { useLocale } from "@/providers/locale-provider";

/** The same read-only capability presentation for saved agents and import candidates. */
export function AgentCapabilityList({
  references,
  capabilities,
}: {
  references: string[];
  capabilities: Capability[];
}) {
  const { locale } = useLocale();
  const capabilityById = new Map(capabilities.map((cap) => [cap.id, cap]));
  return references.length ? (
    <ol className="flex flex-col gap-1" aria-label="Enabled capabilities">
      {references.map((reference) => {
        const cap = capabilityById.get(reference);
        return (
          <li
            key={reference}
            className="flex min-w-0 items-center gap-2 border bg-background p-2 text-[13px]"
          >
            {cap && <CapabilityIcon icon={cap.icon} className="size-3.5 shrink-0" />}
            <span className="min-w-0 flex-1 truncate">
              {cap ? localizedCapabilityName(cap, locale) : reference}
            </span>
            {cap?.is_guardrail && (
              <Badge variant="outline" className="gap-0.5">
                <Shield />
                Guardrail
              </Badge>
            )}
          </li>
        );
      })}
    </ol>
  ) : (
    <p className="text-[13px] text-muted-foreground">No capabilities enabled.</p>
  );
}
