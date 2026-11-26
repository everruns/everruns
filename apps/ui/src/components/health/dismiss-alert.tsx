"use client";

import { useState } from "react";
import { useDismissFinding } from "@/hooks/use-health-issues";
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
import { Textarea } from "@/components/ui/textarea";

export function DismissAlert({ issueId }: { issueId: string }) {
  const ui = useUiTranslations();
  const dismiss = useDismissFinding();
  const [open, setOpen] = useState(false);
  const [reason, setReason] = useState("");
  async function submit() {
    try {
      await dismiss.mutateAsync({ id: issueId, reason: reason.trim() });
      setOpen(false);
      setReason("");
    } catch {
      // Keep the reason and the server's error visible for a retry.
    }
  }
  return (
    <>
      <Button type="button" variant="outline" size="sm" onClick={() => setOpen(true)}>
        {ui("Dismiss alert")}
      </Button>
      <Dialog
        open={open}
        onOpenChange={(value) => {
          if (!dismiss.isPending) setOpen(value);
        }}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{ui("Dismiss alert")}</DialogTitle>
            <DialogDescription>
              {ui(
                "Explain why this needs no action, for example expected work or a false alarm. Dismissal does not stop the agent or change its budget.",
              )}
            </DialogDescription>
          </DialogHeader>
          <Textarea
            aria-label={ui("Reason")}
            value={reason}
            maxLength={500}
            disabled={dismiss.isPending}
            onChange={(event) => setReason(event.target.value)}
          />
          {dismiss.error && (
            <p role="alert" className="text-sm text-destructive">
              {dismiss.error.message}
            </p>
          )}
          <DialogFooter>
            <Button variant="outline" disabled={dismiss.isPending} onClick={() => setOpen(false)}>
              {ui("Cancel")}
            </Button>
            <Button disabled={!reason.trim() || dismiss.isPending} onClick={() => void submit()}>
              {dismiss.isPending ? ui("Working…") : ui("Dismiss")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </>
  );
}
