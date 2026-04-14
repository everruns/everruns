"use client";

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

export function EnqueueDialog({
  open,
  onOpenChange,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const [activityType, setActivityType] = useState("");
  const [inputJson, setInputJson] = useState("{}");
  const [maxAttempts, setMaxAttempts] = useState("3");
  const [priority, setPriority] = useState("0");
  const [jsonError, setJsonError] = useState<string | null>(null);
  const enqueueMutation = useEnqueueTask();

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault();
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
          <DialogTitle>Enqueue Task</DialogTitle>
          <DialogDescription>Add a standalone task to the queue.</DialogDescription>
        </DialogHeader>
        <form onSubmit={handleSubmit} className="space-y-4">
          <div className="space-y-2">
            <Label htmlFor="activity-type">Activity Type</Label>
            <Input
              id="activity-type"
              value={activityType}
              onChange={(e) => setActivityType(e.target.value)}
              placeholder="e.g. send_email"
              required
            />
          </div>
          <div className="space-y-2">
            <Label htmlFor="input-json">Input (JSON)</Label>
            <textarea
              id="input-json"
              value={inputJson}
              onChange={(e) => {
                setInputJson(e.target.value);
                setJsonError(null);
              }}
              className="flex min-h-[100px] w-full border border-input bg-background px-3 py-2 text-sm font-mono ring-offset-background placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
              placeholder='{"key": "value"}'
            />
            {jsonError && <p className="text-sm text-destructive">{jsonError}</p>}
          </div>
          <div className="grid grid-cols-2 gap-4">
            <div className="space-y-2">
              <Label htmlFor="max-attempts">Max Attempts</Label>
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
              <Label htmlFor="priority">Priority</Label>
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
              Failed to enqueue: {enqueueMutation.error.message}
            </p>
          )}
          <DialogFooter>
            <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
              Cancel
            </Button>
            <Button type="submit" disabled={enqueueMutation.isPending || !activityType.trim()}>
              {enqueueMutation.isPending ? "Enqueuing..." : "Enqueue"}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
