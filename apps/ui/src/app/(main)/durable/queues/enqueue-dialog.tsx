"use client";

import { useUiTranslations } from "@/providers/locale-provider";
import { useDurablePageAccess } from "@/components/chat-mode/use-durable-page-rows";

import { z } from "zod";
import { usePageFormDraft } from "@/components/chat-mode/use-page-form-draft";
import { useState } from "react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { useEnqueueTask } from "@/hooks";

const enqueueDraftSchema = z.strictObject({
  activity_type: z.string(),
  max_attempts: z.string().regex(/^(?:[1-9]|10)$/),
  priority: z.string().regex(/^-?(?:[0-9]|10)$/),
});

export function EnqueueDialog({
  open,
  onOpenChange,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const ui = useUiTranslations();
  const [activityType, setActivityType] = useState("");
  const [inputJson, setInputJson] = useState("{}");
  const [maxAttempts, setMaxAttempts] = useState("3");
  const [priority, setPriority] = useState("0");
  const [jsonError, setJsonError] = useState<string | null>(null);
  const enqueueMutation = useEnqueueTask();

  // Payloads may contain credentials. Chat can stage queue metadata, while the
  // task body and the actual enqueue action stay in the manual form.
  const { manage: canManageDraft } = useDurablePageAccess();
  const chat = usePageFormDraft({
    schema: enqueueDraftSchema,
    entityType: "queued_task",
    label: ui("Enqueue task metadata"),
    priority: 30,
    enabled: open,
    values: { activity_type: activityType, max_attempts: maxAttempts, priority },
    baseline: "new",
    dirty: !!activityType || maxAttempts !== "3" || priority !== "0",
    writable: canManageDraft && !enqueueMutation.isPending,
    apply: (next) => {
      setActivityType(next.activity_type);
      setMaxAttempts(next.max_attempts);
      setPriority(next.priority);
    },
  });

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    if (chat.blocked) return;
    try {
      const parsed = JSON.parse(inputJson);
      setJsonError(null);
      enqueueMutation.mutate(
        {
          activity_type: activityType,
          input: parsed,
          max_attempts: parseInt(maxAttempts, 10),
          priority: parseInt(priority, 10),
        },
        {
          onSuccess: () => {
            setActivityType("");
            setInputJson("{}");
            setMaxAttempts("3");
            setPriority("0");
            onOpenChange(false);
          },
        },
      );
    } catch {
      setJsonError("Invalid JSON");
    }
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>{ui("Enqueue Task")}</DialogTitle>
          <DialogDescription>{ui("Add a standalone task to the queue.")}</DialogDescription>
        </DialogHeader>
        <form onSubmit={handleSubmit} className="space-y-4">
          {chat.notice}
          <div className="space-y-2">
            <Label htmlFor="activity-type">{ui("Activity Type")}</Label>
            <Input
              id="activity-type"
              value={activityType}
              onChange={(e) => setActivityType(e.target.value)}
              placeholder={ui("e.g. send_email")}
              required
            />
          </div>
          <div className="space-y-2">
            <Label htmlFor="input-json">{ui("Input (JSON)")}</Label>
            <textarea
              id="input-json"
              value={inputJson}
              onChange={(e) => {
                setInputJson(e.target.value);
                setJsonError(null);
              }}
              className="flex min-h-[100px] w-full border border-input bg-background px-3 py-2 text-sm font-mono ring-offset-background placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
              placeholder={ui('{"key": "value"}')}
              aria-label={ui("Input JSON")}
            />
            {jsonError && <p className="text-sm text-destructive">{jsonError}</p>}
          </div>
          <div className="grid grid-cols-2 gap-4">
            <div className="space-y-2">
              <Label htmlFor="max-attempts">{ui("Max Attempts")}</Label>
              <Input
                id="max-attempts"
                type="number"
                min="1"
                max="10"
                value={maxAttempts}
                onChange={(e) => setMaxAttempts(e.target.value)}
              />
            </div>
            <div className="space-y-2">
              <Label htmlFor="priority">{ui("Priority")}</Label>
              <Input
                id="priority"
                type="number"
                min="-10"
                max="10"
                value={priority}
                onChange={(e) => setPriority(e.target.value)}
              />
            </div>
          </div>
          {enqueueMutation.isError && (
            <p className="text-sm text-destructive">
              {ui("Failed to enqueue:")} {enqueueMutation.error.message}
            </p>
          )}
          <DialogFooter>
            <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
              {ui("Cancel")}
            </Button>
            <Button
              type="submit"
              disabled={chat.blocked || enqueueMutation.isPending || !activityType.trim()}
            >
              {enqueueMutation.isPending ? ui("Enqueuing...") : ui("Enqueue")}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
