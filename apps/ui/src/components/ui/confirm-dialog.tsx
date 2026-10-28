"use client";
import { useState, type ReactNode } from "react";
import { Button } from "./button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "./dialog";
import { useUiTranslations } from "@/providers/locale-provider";

export function ConfirmDialog({
  open,
  onOpenChange,
  title,
  description,
  confirmLabel,
  onConfirm,
  isPending = false,
  disabled = false,
  destructive = true,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  title: string;
  description: ReactNode;
  confirmLabel: string;
  onConfirm: () => Promise<unknown>;
  isPending?: boolean;
  disabled?: boolean;
  destructive?: boolean;
}) {
  const ui = useUiTranslations();
  const [pending, setPending] = useState(false);
  const [failed, setFailed] = useState(false);
  const busy = pending || isPending;
  async function confirm() {
    if (busy) return;
    setPending(true);
    setFailed(false);
    try {
      await onConfirm();
      onOpenChange(false);
    } catch {
      setFailed(true);
    } finally {
      setPending(false);
    }
  }
  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        if (!busy) {
          setFailed(false);
          onOpenChange(next);
        }
      }}
    >
      <DialogContent chatProtected="Confirm action" showCloseButton={!busy}>
        <DialogHeader>
          <DialogTitle>{title}</DialogTitle>
          <DialogDescription>{description}</DialogDescription>
        </DialogHeader>
        {failed && (
          <p role="alert" className="text-sm text-destructive">
            {ui("The request could not be completed. Please try again.")}
          </p>
        )}
        <DialogFooter>
          <Button variant="outline" disabled={busy} onClick={() => onOpenChange(false)}>
            {ui("Cancel")}
          </Button>
          <Button
            variant={destructive ? "destructive" : "default"}
            loading={busy}
            disabled={disabled}
            onClick={() => void confirm()}
          >
            {confirmLabel}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
