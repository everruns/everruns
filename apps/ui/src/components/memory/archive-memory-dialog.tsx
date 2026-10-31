"use client";

import { useUiTranslations } from "@/providers/locale-provider";
import { ConfirmDialog } from "@/components/ui/confirm-dialog";
import type { Memory } from "@/lib/api/types";

interface ArchiveMemoryDialogProps {
  memory: Memory | null;
  open: boolean;
  isPending?: boolean;
  onOpenChange: (open: boolean) => void;
  onArchive: () => Promise<void>;
}

export function ArchiveMemoryDialog({
  memory,
  open,
  isPending = false,
  onOpenChange,
  onArchive,
}: ArchiveMemoryDialogProps) {
  const ui = useUiTranslations();
  return (
    <ConfirmDialog
      open={open}
      onOpenChange={onOpenChange}
      title={ui("Archive Memory")}
      description={memory ? `Archive ${memory.name}?` : ui("Archive this memory?")}
      confirmLabel={ui("Archive")}
      isPending={isPending}
      disabled={!memory}
      onConfirm={onArchive}
    />
  );
}
