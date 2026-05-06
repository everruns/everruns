"use client";

import { useEffect, useState, type FormEvent } from "react";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import type { CreateVolumeRequest, UpdateVolumeRequest, Volume } from "@/lib/api/types";

type VolumeFormMode = "create" | "edit";

interface VolumeFormDialogProps {
  mode: VolumeFormMode;
  open: boolean;
  volume?: Volume | null;
  isPending?: boolean;
  onOpenChange: (open: boolean) => void;
  onSubmit: (request: CreateVolumeRequest | UpdateVolumeRequest) => Promise<void>;
}

export function VolumeFormDialog({
  mode,
  open,
  volume,
  isPending = false,
  onOpenChange,
  onSubmit,
}: VolumeFormDialogProps) {
  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  const [fieldError, setFieldError] = useState<string | null>(null);
  const [formError, setFormError] = useState<string | null>(null);

  useEffect(() => {
    if (!open) {
      return;
    }
    setName(mode === "edit" ? (volume?.name ?? "") : "");
    setDescription(mode === "edit" ? (volume?.description ?? "") : "");
    setFieldError(null);
    setFormError(null);
  }, [mode, open, volume]);

  async function handleSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    setFieldError(null);
    setFormError(null);

    const trimmedName = name.trim();
    if (!trimmedName) {
      setFieldError("Name is required");
      return;
    }

    try {
      const trimmedDescription = description.trim();
      if (mode === "create") {
        await onSubmit({
          name: trimmedName,
          ...(trimmedDescription ? { description: trimmedDescription } : {}),
        });
      } else {
        await onSubmit({
          name: trimmedName,
          description: trimmedDescription || null,
        });
      }
      onOpenChange(false);
    } catch (error) {
      setFormError(error instanceof Error ? error.message : "Volume could not be saved");
    }
  }

  const title = mode === "create" ? "New Volume" : "Edit Volume";
  const submitLabel = mode === "create" ? "Create Volume" : "Save Changes";

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>{title}</DialogTitle>
          <DialogDescription>
            {mode === "create" ? "Create an org-scoped workspace volume." : volume?.id}
          </DialogDescription>
        </DialogHeader>
        <form onSubmit={handleSubmit} className="space-y-4">
          <div className="space-y-2">
            <Label htmlFor="volume-name">Name</Label>
            <Input
              id="volume-name"
              value={name}
              onChange={(event) => {
                setName(event.target.value);
                setFieldError(null);
              }}
              aria-invalid={!!fieldError}
              required
            />
            {fieldError && <p className="text-xs text-destructive">{fieldError}</p>}
          </div>
          <div className="space-y-2">
            <Label htmlFor="volume-description">Description</Label>
            <Textarea
              id="volume-description"
              value={description}
              onChange={(event) => setDescription(event.target.value)}
              rows={3}
            />
          </div>
          {formError && <p className="text-sm text-destructive">{formError}</p>}
          <DialogFooter>
            <Button
              type="button"
              variant="outline"
              onClick={() => onOpenChange(false)}
              disabled={isPending}
            >
              Cancel
            </Button>
            <Button type="submit" variant="accent" disabled={isPending}>
              {submitLabel}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
