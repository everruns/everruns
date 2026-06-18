"use client";

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
  async function handleArchive() {
    await onArchive();
    onOpenChange(false);
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Archive Knowledge Index</DialogTitle>
          <DialogDescription>
            {index ? `Archive ${index.name}?` : "Archive this knowledge index?"}
          </DialogDescription>
        </DialogHeader>
        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)} disabled={isPending}>
            Cancel
          </Button>
          <Button variant="destructive" onClick={handleArchive} disabled={isPending || !index}>
            Archive
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
