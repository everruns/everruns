"use client";

import { useUiTranslations } from "@/providers/locale-provider";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
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
  async function handleArchive() {
    await onArchive();
    onOpenChange(false);
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>{ui("Archive Knowledge Index")}</DialogTitle>
          <DialogDescription>
            {index
              ? ui("Archive {value0}?", { value0: index.name })
              : ui("Archive this knowledge index?")}
          </DialogDescription>
        </DialogHeader>
        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)} disabled={isPending}>
            {ui("Cancel")}
          </Button>
          <Button variant="destructive" onClick={handleArchive} disabled={isPending || !index}>
            {ui("Archive")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
