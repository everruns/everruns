"use client";

import { useState, useCallback } from "react";
import type { FileInfo } from "@/lib/api/types";
import { FileBrowser, FileViewer } from "@/components/files";
import { useSessionContext } from "../session-context";
import { File, Upload, Loader2, CheckCircle2, XCircle } from "lucide-react";
import { useFileDropUpload } from "@/hooks/use-file-drop-upload";

export default function FilesPage() {
  const { sessionId } = useSessionContext();
  const [selectedFile, setSelectedFile] = useState<FileInfo | null>(null);
  const [refreshToken, setRefreshToken] = useState(0);

  const handleUploadComplete = useCallback(() => {
    setRefreshToken((n) => n + 1);
  }, []);

  const { isDragging, uploads, isUploading, dragHandlers } = useFileDropUpload(
    sessionId,
    "/workspace",
    handleUploadComplete,
  );

  return (
    <div className="flex-1 flex overflow-hidden relative" {...dragHandlers}>
      {/* File browser sidebar */}
      <div className="w-72 border-r flex-shrink-0 overflow-hidden bg-card/50">
        <FileBrowser
          sessionId={sessionId}
          onFileSelect={setSelectedFile}
          selectedPath={selectedFile?.path}
          refreshToken={refreshToken}
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

      {/* Drag overlay */}
      {isDragging && (
        <div className="absolute inset-0 z-50 flex items-center justify-center bg-background/80 backdrop-blur-sm border-2 border-dashed border-primary/50 rounded-lg pointer-events-none">
          <div className="flex flex-col items-center gap-3 text-primary">
            <Upload className="size-12" />
            <p className="text-lg font-medium">Drop files to upload</p>
            <p className="text-sm text-muted-foreground">Files will be added to /workspace</p>
          </div>
        </div>
      )}

      {/* Upload progress toast */}
      {uploads.length > 0 && (
        <div className="absolute bottom-4 right-4 z-50 w-72 bg-card border rounded-lg shadow-lg p-3 space-y-2">
          <p className="text-sm font-medium">
            {isUploading ? "Uploading files..." : "Upload complete"}
          </p>
          {uploads.map((item) => (
            <div key={item.name} className="flex items-center gap-2 text-xs">
              {item.status === "done" ? (
                <CheckCircle2 className="size-3.5 text-green-500 flex-shrink-0" />
              ) : item.status === "error" ? (
                <XCircle className="size-3.5 text-destructive flex-shrink-0" />
              ) : (
                <Loader2 className="size-3.5 animate-spin flex-shrink-0" />
              )}
              <span className="truncate">{item.name}</span>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
