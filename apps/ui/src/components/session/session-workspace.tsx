"use client";

// Workspace — the state the run left behind (EVE-854): the session's files,
// its key values, and the schedules it registered or deleted. All read-only.
// Files and values are the pair `SessionSeedMode::Fork` copies.
//
// The tab used to accept drag-and-drop uploads, inline file edits and secret
// writes. A recording is not a workspace, so all of that is gone; the browser
// and viewer render in `readOnly` mode and storage is a listing.
//
// Key values used to sit under the file viewer, and only when the session
// also had a filesystem. A sandbox or storage session never opened this tab,
// so the values (and the sandbox's files) were unreachable.

import { useUiTranslations, useLocale } from "@/providers/locale-provider";
import { useState } from "react";
import { Clock, File, Key, Lock } from "lucide-react";
import type { FileInfo, KeyValueInfo, ListResponse, SecretInfo } from "@/lib/api/types";
import { FileBrowser, FileViewer, useWorkspaceFileSource } from "@/components/files";
import { Badge } from "@/components/ui/badge";
import { Skeleton } from "@/components/ui/skeleton";
import { api } from "@/lib/api/client";
import { useQuery } from "@tanstack/react-query";
import { SessionSandboxPanel } from "@/components/session/session-sandbox-panel";
import { SessionSchedulesSection } from "@/components/session/session-schedule-list";
import { sessionRecordPanes, type SessionRecordPane } from "@/components/session/session-header";
import { useSessionSchedules } from "@/hooks/use-session-schedules";
import { useSessionContext } from "@/app/(main)/sessions/[sessionId]/session-context";
import { cn } from "@/lib/utils";

function useSessionStorage(sessionId: string) {
  return useQuery({
    queryKey: ["session-storage", sessionId],
    queryFn: async () => {
      const [keys, secrets] = await Promise.all([
        api.get<ListResponse<KeyValueInfo>>(`/v1/sessions/${sessionId}/storage/keys`),
        api.get<ListResponse<SecretInfo>>(`/v1/sessions/${sessionId}/storage/secrets`),
      ]);
      return { keyValues: keys.data.data, secrets: secrets.data.data };
    },
  });
}

function StoragePanel({ sessionId }: { sessionId: string }) {
  const { displayLocale } = useLocale();

  const ui = useUiTranslations();
  const { data, isLoading } = useSessionStorage(sessionId);

  return (
    <div className="p-4">
      <h2 className="mb-3 text-sm font-medium text-muted-foreground">{ui("Session storage")}</h2>

      {isLoading ? (
        <Skeleton className="h-24 w-full" />
      ) : (
        <div className="space-y-4">
          <div>
            <div className="mb-2 flex items-center gap-1.5 text-xs font-medium text-muted-foreground">
              <Key className="h-3 w-3" />
              {ui("Key-value")}
            </div>
            {!data || data.keyValues.length === 0 ? (
              <p className="text-sm text-muted-foreground">{ui("Nothing stored.")}</p>
            ) : (
              <div className="space-y-2">
                {data.keyValues.map((entry) => (
                  <div key={entry.key} className="space-y-1 border p-3">
                    <div className="flex items-center justify-between gap-2">
                      <Badge variant="outline" className="font-mono">
                        {entry.key}
                      </Badge>
                      <span className="flex items-center gap-1 text-xs text-muted-foreground">
                        <Clock className="h-3 w-3" />
                        {new Date(entry.updated_at).toLocaleString(displayLocale)}
                      </span>
                    </div>
                    <pre className="overflow-x-auto whitespace-pre-wrap break-all rounded bg-muted p-2 text-sm">
                      {entry.value}
                    </pre>
                  </div>
                ))}
              </div>
            )}
          </div>

          <div>
            <div className="mb-2 flex items-center gap-1.5 text-xs font-medium text-muted-foreground">
              <Lock className="h-3 w-3" />
              {ui("Secrets")}
            </div>
            {!data || data.secrets.length === 0 ? (
              <p className="text-sm text-muted-foreground">{ui("No secrets stored.")}</p>
            ) : (
              <div className="space-y-2">
                {data.secrets.map((secret) => (
                  <div
                    key={secret.name}
                    className="flex items-center justify-between gap-3 border p-3"
                  >
                    <Badge variant="secondary" className="font-mono">
                      {secret.name}
                    </Badge>
                    <span className="flex items-center gap-1 text-xs text-muted-foreground">
                      <Clock className="h-3 w-3" />
                      {new Date(secret.updated_at).toLocaleString(displayLocale)}
                    </span>
                  </div>
                ))}
              </div>
            )}
          </div>
        </div>
      )}
    </div>
  );
}

const PANE_LABELS: Record<SessionRecordPane, string> = {
  files: "Filesystem",
  values: "Key values",
  schedules: "Schedules",
};

function FilesystemPane({
  workspaceId,
  sessionId,
}: {
  workspaceId: string | undefined;
  sessionId: string;
}) {
  const ui = useUiTranslations();
  const [selectedFile, setSelectedFile] = useState<FileInfo | null>(null);
  const source = useWorkspaceFileSource(workspaceId, { readOnly: true });

  return (
    <div className="relative flex min-h-0 flex-1 overflow-hidden">
      <div className="w-72 flex-shrink-0 overflow-hidden border-r bg-card/50">
        <FileBrowser
          key={source.key}
          source={source}
          onFileSelect={setSelectedFile}
          selectedPath={selectedFile?.path}
        />
      </div>

      <div className="flex flex-1 flex-col overflow-y-auto">
        <div className="min-h-0 flex-1 overflow-hidden">
          {selectedFile && !selectedFile.is_directory ? (
            <FileViewer source={source} file={selectedFile} onClose={() => setSelectedFile(null)} />
          ) : (
            <div className="flex h-full flex-col items-center justify-center py-16 text-muted-foreground">
              <File className="mb-4 size-12 opacity-30" />
              <p className="text-sm font-medium">{ui("No file selected")}</p>
              <p className="mt-1 text-xs">
                {ui("Select a file from the sidebar to view its contents")}
              </p>
            </div>
          )}
        </div>

        <SessionSandboxPanel sessionId={sessionId} />
      </div>
    </div>
  );
}

export function SessionWorkspace() {
  const ui = useUiTranslations();
  // Address files by the session's attached workspace (the canonical fs
  // surface), not the session id — they differ when the session is attached to
  // a shared workspace. `workspace_id` is undefined only while the session is
  // still loading.
  const { session, sessionId } = useSessionContext();
  const workspaceId = session?.workspace_id;
  const features = new Set(session?.features ?? []);
  const schedules = useSessionSchedules(sessionId);
  const storage = useSessionStorage(sessionId);
  const hasScheduleRows = (schedules.data?.length ?? 0) > 0;
  const hasValueRows =
    (storage.data?.keyValues.length ?? 0) > 0 || (storage.data?.secrets.length ?? 0) > 0;
  const panes = sessionRecordPanes({
    features,
    hasSchedules: hasScheduleRows,
    hasValues: hasValueRows,
  });
  const [pane, setPane] = useState<SessionRecordPane>(panes[0] ?? "files");
  const active = panes.includes(pane) ? pane : (panes[0] ?? "files");

  return (
    <div className="flex min-h-0 flex-1 flex-col overflow-hidden">
      {panes.length > 1 && (
        <div className="flex border-b" role="tablist" aria-label={ui("Session state")}>
          {panes.map((item) => (
            <button
              key={item}
              type="button"
              role="tab"
              aria-selected={active === item}
              onClick={() => setPane(item)}
              className={cn(
                "px-4 py-2 text-sm",
                active === item
                  ? "border-b-2 border-foreground text-foreground"
                  : "text-muted-foreground hover:text-foreground",
              )}
            >
              {PANE_LABELS[item]}
            </button>
          ))}
        </div>
      )}

      {active === "files" && <FilesystemPane workspaceId={workspaceId} sessionId={sessionId} />}
      {active === "values" && (
        <div className="flex-1 overflow-y-auto">
          <StoragePanel sessionId={sessionId} />
        </div>
      )}
      {active === "schedules" && (
        <div className="flex-1 overflow-y-auto p-6">
          <SessionSchedulesSection sessionId={sessionId} announceEmpty showHeading={false} />
        </div>
      )}
    </div>
  );
}
