"use client";

import { useState } from "react";
import type { FileInfo } from "@/lib/api/types";
import { FileBrowser, FileViewer } from "@/components/files";
import { useSessionContext } from "../session-context";
import { File } from "lucide-react";

export default function FilesPage() {
  const { sessionId } = useSessionContext();
  const [selectedFile, setSelectedFile] = useState<FileInfo | null>(null);

  return (
    <div className="flex-1 flex overflow-hidden">
      {/* File browser sidebar */}
      <div className="w-72 border-r flex-shrink-0 overflow-hidden bg-card/50">
        <FileBrowser
          sessionId={sessionId}
          onFileSelect={setSelectedFile}
          selectedPath={selectedFile?.path}
        />
      </div>

      {/* File viewer main area */}
      <div className="flex-1 overflow-hidden">
        {selectedFile && !selectedFile.is_directory ? (
          <FileViewer
            sessionId={sessionId}
            file={selectedFile}
            onClose={() => setSelectedFile(null)}
          />
        ) : (
          <div className="flex flex-col items-center justify-center h-full text-muted-foreground">
            <File className="size-12 mb-4 opacity-30" />
            <p className="text-sm font-medium">No file selected</p>
            <p className="text-xs mt-1">Select a file from the sidebar to view its contents</p>
          </div>
        )}
      </div>
    </div>
  );
}
