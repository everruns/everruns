"use client";

import { useUiTranslations } from "@/providers/locale-provider";
import { ConfirmDialog } from "@/components/ui/confirm-dialog";
import type { KnowledgeIndex } from "@/lib/api/types";

interface ArchiveKnowledgeIndexDialogProps {
  index: KnowledgeIndex | null;
  open: boolean;
  isPending?: boolean;
  onOpenChange: (open: boolean) => void;
  onArchive: () => Promise<void>;
}

export function ArchiveKnowledgeIndexDialog({
  index,
  open,
  isPending = false,
  onOpenChange,
  onArchive,
}: ArchiveKnowledgeIndexDialogProps) {
  const ui = useUiTranslations();
  return (
    <ConfirmDialog
      open={open}
      onOpenChange={onOpenChange}
      title={ui("Archive Knowledge Index")}
      description={index ? `Archive ${index.name}?` : ui("Archive this knowledge index?")}
      confirmLabel={ui("Archive")}
      isPending={isPending}
      disabled={!index}
      onConfirm={onArchive}
    />
  );
}
