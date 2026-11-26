"use client";

// Create or rename a named memory. A named memory is a store agents reach at
// /memory/<name> once it is attached in their Memory capability, so its name
// is a store name: lower-case letters, digits and dashes. Source-backed
// (GitHub/Git) memories were retired with memory v1.

import { useUiTranslations } from "@/providers/locale-provider";
import { useState, type FormEvent } from "react";
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
import type { CreateMemoryRequest, UpdateMemoryRequest, Memory } from "@/lib/api/types";

type MemoryFormMode = "create" | "edit";

const RESERVED = ["agent", "user", "org", "team", "parent", "named", "shared"];

/** Why `name` cannot name a store, or null when it can (mirrors `validate_store_name`). */
export function storeNameError(name: string): string | null {
  if (!/^[a-z]([a-z0-9-]{0,61}[a-z0-9])?$/.test(name)) {
    return "Use up to 63 lower-case letters, digits and dashes, starting with a letter.";
  }
  if (RESERVED.includes(name)) {
    return `"${name}" is the name of a built-in store.`;
  }
  return null;
}

interface MemoryFormDialogProps {
  mode: MemoryFormMode;
  open: boolean;
  memory?: Memory | null;
  isPending?: boolean;
  writable?: boolean;
  onOpenChange: (open: boolean) => void;
  onSubmit: (request: CreateMemoryRequest | UpdateMemoryRequest) => Promise<void>;
}

export function MemoryFormDialog(props: MemoryFormDialogProps) {
  // The form remounts each time the dialog opens, so it starts from the
  // memory being edited (or empty) without syncing state in an effect.
  return (
    <Dialog open={props.open} onOpenChange={props.onOpenChange}>
      {props.open && <MemoryForm key={props.memory?.id ?? "new"} {...props} />}
    </Dialog>
  );
}

function MemoryForm({
  mode,
  memory,
  isPending = false,
  writable = false,
  onOpenChange,
  onSubmit,
}: MemoryFormDialogProps) {
  const ui = useUiTranslations();
  const [name, setName] = useState(mode === "edit" ? (memory?.name ?? "") : "");
  const [description, setDescription] = useState(
    mode === "edit" ? (memory?.description ?? "") : "",
  );
  const [fieldError, setFieldError] = useState<string | null>(null);
  const [formError, setFormError] = useState<string | null>(null);

  async function handleSubmit(event: FormEvent) {
    event.preventDefault();
    const trimmed = name.trim();
    const error = storeNameError(trimmed);
    if (error) {
      setFieldError(error);
      return;
    }
    setFormError(null);
    try {
      await onSubmit({
        name: trimmed,
        description: description.trim() || undefined,
      });
      onOpenChange(false);
    } catch (err) {
      setFormError(err instanceof Error ? err.message : String(err));
    }
  }

  return (
    <DialogContent>
      <form onSubmit={handleSubmit} className="space-y-4">
        <DialogHeader>
          <DialogTitle>
            {mode === "create" ? ui("New named memory") : ui("Edit memory")}
          </DialogTitle>
          <DialogDescription>
            {ui(
              "A named memory is shared by every agent you attach it to in the Memory capability. Agents find it at /memory/<name>.",
            )}
          </DialogDescription>
        </DialogHeader>
        <div className="space-y-2">
          <Label htmlFor="memory-name">{ui("Name")}</Label>
          <Input
            id="memory-name"
            value={name}
            placeholder="everruns-maintenance"
            disabled={!writable || isPending}
            aria-invalid={fieldError ? true : undefined}
            onChange={(event) => {
              setName(event.target.value);
              setFieldError(null);
            }}
          />
          {fieldError ? (
            <p className="text-xs text-destructive">{ui(fieldError)}</p>
          ) : (
            <p className="font-mono text-xs text-muted-foreground">
              /memory/{name.trim() || "<name>"}
            </p>
          )}
        </div>
        <div className="space-y-2">
          <Label htmlFor="memory-description">{ui("Description")}</Label>
          <Textarea
            id="memory-description"
            value={description}
            rows={3}
            placeholder={ui("What the agents sharing it should keep here")}
            disabled={!writable || isPending}
            onChange={(event) => setDescription(event.target.value)}
          />
        </div>
        {formError && <p className="text-sm text-destructive">{formError}</p>}
        <DialogFooter>
          <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
            {ui("Cancel")}
          </Button>
          <Button type="submit" variant="accent" disabled={!writable || isPending}>
            {mode === "create" ? ui("Create") : ui("Save")}
          </Button>
        </DialogFooter>
      </form>
    </DialogContent>
  );
}
