"use client";

// Workspace — the state the run left behind (EVE-854): the session's files and
// its durable session storage, both read-only. These two are shown together
// because they are the pair `SessionSeedMode::Fork` copies: forking a recording
// carries exactly what this tab shows.
//
// The tab used to accept drag-and-drop uploads, inline file edits and secret
// writes. A recording is not a workspace, so all of that is gone; the browser
// and viewer render in `readOnly` mode and storage is a listing.

import { useState } from "react";
import { Clock, File, Key, Lock } from "lucide-react";
import type { FileInfo, KeyValueInfo, ListResponse, SecretInfo } from "@/lib/api/types";
import { FileBrowser, FileViewer } from "@/components/files";
import { Badge } from "@/components/ui/badge";
import { Skeleton } from "@/components/ui/skeleton";
import { api } from "@/lib/api/client";
import { useQuery } from "@tanstack/react-query";
import { useSessionContext } from "../session-context";

function useSessionStorage(sessionId: string, enabled: boolean) {
  return useQuery({
    queryKey: ["session-storage", sessionId],
    enabled,
    queryFn: async () => {
      const [keys, secrets] = await Promise.all([
        api.get<ListResponse<KeyValueInfo>>(`/v1/sessions/${sessionId}/storage/keys`),
        api.get<ListResponse<SecretInfo>>(`/v1/sessions/${sessionId}/storage/secrets`),
      ]);
      return { keyValues: keys.data.data, secrets: secrets.data.data };
    },
  });
}

function StoragePanel({ sessionId, enabled }: { sessionId: string; enabled: boolean }) {
  const { data, isLoading } = useSessionStorage(sessionId, enabled);

  if (!enabled) return null;

  return (
    <div className="border-t p-4">
      <h2 className="mb-3 text-sm font-medium text-muted-foreground">Session storage</h2>

      {isLoading ? (
        <Skeleton className="h-24 w-full" />
      ) : (
        <div className="space-y-4">
          <div>
            <div className="mb-2 flex items-center gap-1.5 text-xs uppercase tracking-[0.14em] text-muted-foreground">
              <Key className="h-3 w-3" />
              Key-value
            </div>
            {!data || data.keyValues.length === 0 ? (
              <p className="text-sm text-muted-foreground">Nothing stored.</p>
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
                        {new Date(entry.updated_at).toLocaleString()}
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
            <div className="mb-2 flex items-center gap-1.5 text-xs uppercase tracking-[0.14em] text-muted-foreground">
              <Lock className="h-3 w-3" />
              Secrets
            </div>
            {!data || data.secrets.length === 0 ? (
              <p className="text-sm text-muted-foreground">No secrets stored.</p>
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
                      {new Date(secret.updated_at).toLocaleString()}
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

export default function WorkspacePage() {
  // Address files by the session's attached workspace (the canonical fs
  // surface), not the session id — they differ when the session is attached to
  // a shared workspace. `workspace_id` is undefined only while the session is
  // still loading.
  const { session, sessionId } = useSessionContext();
  const workspaceId = session?.workspace_id;
  const [selectedFile, setSelectedFile] = useState<FileInfo | null>(null);

  const features = new Set(session?.features ?? []);
  const hasStorage = features.has("secrets") || features.has("key_value");

  return (
    <div className="relative flex flex-1 overflow-hidden">
      {/* File browser sidebar */}
      <div className="w-72 flex-shrink-0 overflow-hidden border-r bg-card/50">
        <FileBrowser
          workspaceId={workspaceId}
          onFileSelect={setSelectedFile}
          selectedPath={selectedFile?.path}
          readOnly
        />
      </div>

      {/* File viewer main area */}
      <div className="flex flex-1 flex-col overflow-y-auto">
        <div className="min-h-0 flex-1">
          {selectedFile && !selectedFile.is_directory ? (
            <FileViewer
              workspaceId={workspaceId}
              file={selectedFile}
              onClose={() => setSelectedFile(null)}
              readOnly
            />
          ) : (
            <div className="flex h-full flex-col items-center justify-center py-16 text-muted-foreground">
              <File className="mb-4 size-12 opacity-30" />
              <p className="text-sm font-medium">No file selected</p>
              <p className="mt-1 text-xs">Select a file from the sidebar to view its contents</p>
            </div>
          )}
        </div>

        <StoragePanel sessionId={sessionId} enabled={hasStorage} />
      </div>
    </div>
  );
}
