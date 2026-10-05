"use client";

import { useEffect, useState } from "react";
import { inspectAgentPackage, type AgentPackagePreview } from "@/lib/api/agents";
import { AgentPackageReview, PackageChangeReview } from "./agent-package-review";
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
  const [preview, setPreview] = useState<AgentPackagePreview | null>(null);
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
    setPreview(null);
    setChanges(null);
    void (async () => {
      try {
        const validation = await inspectAgentPackage(file, "validate", target);
        if (!current) return;
        setPreview(validation.preview ?? null);
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
      <DialogContent className="flex max-h-[90vh] flex-col gap-0 overflow-hidden p-0 sm:max-w-6xl">
        <DialogHeader className="shrink-0 border-b p-4 pr-12 sm:px-6">
          <DialogTitle>Import agent</DialogTitle>
          <DialogDescription>
            Review {file?.name} before importing. Starting files affect new sessions; existing
            session files stay intact.
          </DialogDescription>
        </DialogHeader>
        <div className="min-h-0 overflow-y-auto">
          <div className="grid gap-3 border-b bg-muted/30 p-4 sm:px-6">
            <label className="grid gap-2 text-sm sm:grid-cols-[auto_minmax(0,320px)] sm:items-center sm:justify-start">
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
          </div>
          {preview && !busy && (
            <AgentPackageReview
              key={`${file?.name}:${target}`}
              preview={preview}
              changes={changes}
            />
          )}
          {!preview && changes && (
            <div className="p-4">
              <PackageChangeReview changes={changes} />
            </div>
          )}
        </div>
        <DialogFooter className="shrink-0 border-t bg-background p-4 sm:px-6">
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
