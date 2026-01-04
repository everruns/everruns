"use client";

import { useState } from "react";
import type { FileInfo } from "@/lib/api/types";
import { FileBrowser, FileViewer } from "@/components/files";
import { useSessionContext } from "../session-context";

export default function FilesPage() {
  const { agentId, sessionId } = useSessionContext();
  const [selectedFile, setSelectedFile] = useState<FileInfo | null>(null);

  return (
    <div className="flex-1 flex overflow-hidden">
      <div className="w-1/3 border-r overflow-y-auto">
        <FileBrowser
          agentId={agentId}
          sessionId={sessionId}
          onFileSelect={setSelectedFile}
          selectedPath={selectedFile?.path}
        />
      </div>
      <div className="flex-1 overflow-y-auto">
        {selectedFile && !selectedFile.is_directory ? (
          <FileViewer
            agentId={agentId}
            sessionId={sessionId}
            file={selectedFile}
            onClose={() => setSelectedFile(null)}
          />
        ) : (
          <div className="flex items-center justify-center h-full text-muted-foreground">
            <p>Select a file to view its contents</p>
          </div>
        )}
      </div>
    </div>
  );
}
