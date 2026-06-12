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
  async function handleArchive() {
    await onArchive();
    onOpenChange(false);
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Archive Memory</DialogTitle>
          <DialogDescription>
            {memory ? `Archive ${memory.name}?` : "Archive this memory?"}
          </DialogDescription>
        </DialogHeader>
        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)} disabled={isPending}>
            Cancel
          </Button>
          <Button variant="destructive" onClick={handleArchive} disabled={isPending || !memory}>
            Archive
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
