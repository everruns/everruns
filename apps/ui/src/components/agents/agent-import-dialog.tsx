"use client";

import { useEffect, useState } from "react";
import { inspectAgentPackage } from "@/lib/api/agents";
import { useImportAgent } from "@/hooks/use-agents";
import type { Agent } from "@/lib/api/types";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
  DialogFooter,
} from "@/components/ui/dialog";

/** The same candidate bytes are validated, compared and imported. */
export function AgentImportDialog({
  file,
  agents,
  onClose,
  onImported,
}: {
  file: File | null;
  agents: Agent[];
  onClose: () => void;
  onImported: (agent: Agent) => void;
}) {
  const importAgent = useImportAgent();
  const [target, setTarget] = useState("");
  const [busy, setBusy] = useState(false);
  const [valid, setValid] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [changes, setChanges] = useState<
    { path: string; before: unknown; after: unknown }[] | null
  >(null);
  useEffect(() => {
    setTarget("");
  }, [file]);
  useEffect(() => {
    if (!file) return;
    let current = true;
    setBusy(true);
    setValid(false);
    setError(null);
    setChanges(null);
    void (async () => {
      try {
        const validation = await inspectAgentPackage(file, "validate", target);
        if (!current) return;
        if (!validation.valid) {
          setError(
            validation.diagnostics?.map((d) => `${d.path}: ${d.message}`).join("\n") ||
              "Invalid package",
          );
          return;
        }
        if (target) {
          const diff = await inspectAgentPackage(file, "diff", target);
          if (!current) return;
          setChanges(diff.changes ?? []);
        }
        setValid(true);
      } catch (err) {
        if (current) setError(err instanceof Error ? err.message : "Validation failed");
      } finally {
        if (current) setBusy(false);
      }
    })();
    return () => {
      current = false;
    };
  }, [file, target]);
  const apply = async () => {
    if (!file || !valid || busy) return;
    setBusy(true);
    setError(null);
    try {
      onImported(await importAgent.mutateAsync({ file, target: target || undefined }));
      onClose();
    } catch (err) {
      setError(err instanceof Error ? err.message : "Import failed");
    } finally {
      setBusy(false);
    }
  };
  return (
    <Dialog
      open={!!file}
      onOpenChange={(open) => {
        if (!open && !busy) onClose();
      }}
    >
      <DialogContent className="max-h-[85vh] overflow-y-auto">
        <DialogHeader>
          <DialogTitle>Import agent</DialogTitle>
          <DialogDescription>
            {file?.name}. Files and complete skills are included. Channels default to disabled;
            enablement creates a draft.
          </DialogDescription>
        </DialogHeader>
        <label className="grid gap-2 text-sm">
          Destination
          <select
            aria-label="Import destination"
            className="rounded border bg-background p-2"
            value={target}
            disabled={busy}
            onChange={(e) => setTarget(e.target.value)}
          >
            <option value="">Create a new agent</option>
            {agents.map((agent) => (
              <option key={agent.id} value={agent.name}>
                Update {agent.display_name || agent.name}
              </option>
            ))}
          </select>
        </label>
        {busy && (
          <p role="status" className="text-sm text-muted-foreground">
            Checking package…
          </p>
        )}
        {error && (
          <p role="alert" className="whitespace-pre-wrap break-words text-sm text-destructive">
            {error}
          </p>
        )}
        {valid && !busy && (
          <p className="text-sm">Package and destination dependencies are valid.</p>
        )}
        {changes && (
          <div className="grid gap-2 text-sm">
            <p>
              {changes.length} {changes.length === 1 ? "change" : "changes"}
            </p>
            {changes.map((change) => (
              <details key={change.path}>
                <summary className="break-all font-mono">{change.path}</summary>
                <pre className="overflow-auto whitespace-pre-wrap break-all rounded bg-muted p-2 text-xs">
                  {JSON.stringify({ before: change.before, after: change.after }, null, 2)}
                </pre>
              </details>
            ))}
          </div>
        )}
        <DialogFooter>
          <Button variant="outline" disabled={busy} onClick={onClose}>
            Cancel
          </Button>
          <Button disabled={!valid || busy} onClick={apply}>
            {target ? "Apply changes" : "Import"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
