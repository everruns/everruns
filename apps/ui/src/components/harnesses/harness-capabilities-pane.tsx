"use client";

// Capabilities are the harness page: the wide left pane of the workspace.
// A harness is the capability set a session starts from; the system prompt is
// optional and lives in More. Viewing lists the local capabilities at reading
// size; editing turns the same pane into the selector, so add, reorder, and
// configure happen where the list was.

import { Pencil, Shield } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { CapabilitySelector } from "@/components/agents/capability-selector";
import type { HarnessDraft } from "@/components/harnesses/use-harness-draft";
import type { Capability } from "@/lib/api/types";
import { CapabilityIcon } from "@/lib/capability-icons";
import {
  localizedCapabilityDescription,
  localizedCapabilityName,
} from "@/lib/capability-localization";
import { pluralize } from "@/lib/formatting";
import { useLocale } from "@/providers/locale-provider";
import { cn } from "@/lib/utils";

interface HarnessCapabilitiesPaneProps {
  draft: HarnessDraft;
  editing: boolean;
  /** Absent for read-only harnesses: the pane stays a list. */
  onEdit?: () => void;
  allCapabilities: Capability[];
  /** When set, an empty list means inherited capabilities show in Preview. */
  hasParent: boolean;
  className?: string;
}

export function HarnessCapabilitiesPane({
  draft,
  editing,
  onEdit,
  allCapabilities,
  hasParent,
  className,
}: HarnessCapabilitiesPaneProps) {
  const { locale } = useLocale();
  const capabilityById = new Map(allCapabilities.map((cap) => [cap.id, cap]));
  const selected = draft.capabilities;
  const countLabel = `${selected.length} ${pluralize(selected.length, "capability", "capabilities")}`;

  return (
    <section
      aria-label="Capabilities"
      className={cn("flex min-w-0 flex-col bg-background", className)}
    >
      <div className="flex flex-wrap items-center gap-3 border-b px-4 py-2.5 sm:px-6">
        <h2 className="text-sm font-medium">Capabilities</h2>
        <span className="font-mono text-xs text-muted-foreground">{countLabel}</span>
        {!editing && onEdit && (
          <div className="ml-auto">
            <Button variant="outline" size="sm" onClick={onEdit}>
              <Pencil className="size-4" />
              Edit capabilities
            </Button>
          </div>
        )}
      </div>

      {editing ? (
        <div className="flex-1 px-4 py-4 sm:px-6">
          <CapabilitySelector
            capabilities={allCapabilities}
            selected={selected}
            onChange={draft.setCapabilities}
            label="Local capabilities"
          />
        </div>
      ) : (
        <div className="min-h-[240px] flex-1 px-4 py-5 sm:px-6">
          {selected.length === 0 ? (
            <p className="text-sm italic text-muted-foreground">
              {hasParent
                ? "No local capabilities. Inherited ones show in Preview."
                : "No capabilities enabled."}
            </p>
          ) : (
            <ol className="flex flex-col gap-2" aria-label="Enabled capabilities">
              {selected.map((config) => {
                const cap = capabilityById.get(config.ref);
                const description = cap ? localizedCapabilityDescription(cap, locale) : "";
                return (
                  <li
                    key={config.ref}
                    className="flex min-w-0 items-start gap-3 border bg-background p-3"
                  >
                    {cap && <CapabilityIcon icon={cap.icon} className="mt-0.5 size-4 shrink-0" />}
                    <div className="min-w-0 flex-1">
                      <div className="flex items-center gap-2">
                        <p className="min-w-0 truncate text-sm font-medium">
                          {cap ? localizedCapabilityName(cap, locale) : config.ref}
                        </p>
                        {cap?.is_guardrail && (
                          <Badge variant="outline" className="gap-0.5">
                            <Shield />
                            Guardrail
                          </Badge>
                        )}
                      </div>
                      {description && (
                        <p className="mt-0.5 text-[13px] text-muted-foreground">{description}</p>
                      )}
                    </div>
                  </li>
                );
              })}
            </ol>
          )}
        </div>
      )}
    </section>
  );
}
