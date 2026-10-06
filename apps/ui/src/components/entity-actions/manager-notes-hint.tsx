"use client";

import Link from "next/link";
import { usePathname, useSearchParams } from "next/navigation";
import { NotebookPen } from "lucide-react";
import { entityKindLabel, type EntityKind } from "@/components/entity-actions/entity-kinds";
import { useManagerContext } from "@/hooks/use-change-history";

/**
 * The one visible sign that notes exist: a muted header line, shown by a page
 * only in edit mode, linking to the notes sheet. Renders nothing when the
 * entity has no notes or the viewer does not manage it.
 */
export function ManagerNotesHint({
  entityRef,
  kind,
  enabled,
}: {
  entityRef: string;
  kind: EntityKind;
  enabled: boolean;
}) {
  const pathname = usePathname();
  const searchParams = useSearchParams();
  const notes = useManagerContext(entityRef, enabled);
  if (!enabled || !notes.data?.content.trim()) return null;
  const params = new URLSearchParams(searchParams.toString());
  params.set("sheet", "notes");
  return (
    <Link
      href={`${pathname}?${params.toString()}`}
      scroll={false}
      replace
      className="inline-flex items-center gap-1.5 text-muted-foreground underline-offset-2 hover:text-foreground hover:underline"
    >
      <NotebookPen className="size-3.5" />
      This {entityKindLabel(kind)} has manager notes
    </Link>
  );
}
