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
  async function handleArchive() {
    await onArchive();
    onOpenChange(false);
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>{ui("Archive Memory")}</DialogTitle>
          <DialogDescription>
            {memory ? ui("Archive {value0}?", { value0: memory.name }) : ui("Archive this memory?")}
          </DialogDescription>
        </DialogHeader>
        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)} disabled={isPending}>
            {ui("Cancel")}
          </Button>
          <Button variant="destructive" onClick={handleArchive} disabled={isPending || !memory}>
            {ui("Archive")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
