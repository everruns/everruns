"use client";

import { useState } from "react";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { ScrollArea } from "@/components/ui/scroll-area";
import {
  File,
  FileText,
  X,
  Save,
  Edit3,
  Lock,
  Download,
} from "lucide-react";
import { useFile, useUpdateFile } from "@/hooks/use-session-files";
import { formatFileSize, getFileExtension } from "@/lib/api/session-files";
import type { FileInfo } from "@/lib/api/types";

interface FileViewerProps {
  agentId: string;
  sessionId: string;
  file: FileInfo;
  onClose: () => void;
}

export function FileViewer({ agentId, sessionId, file, onClose }: FileViewerProps) {
  const [isEditing, setIsEditing] = useState(false);
  const [editContent, setEditContent] = useState("");

  const { data: fileData, isLoading, refetch } = useFile(agentId, sessionId, file.path);
  const updateFile = useUpdateFile();

  const handleEdit = () => {
    if (fileData?.content) {
      setEditContent(fileData.content);
      setIsEditing(true);
    }
  };

  const handleSave = async () => {
    try {
      await updateFile.mutateAsync({
        agentId,
        sessionId,
        path: file.path,
        request: {
          content: editContent,
          encoding: "text",
        },
      });
      setIsEditing(false);
      refetch();
    } catch {
      // Error handling done by react-query
    }
  };

  const handleCancel = () => {
    setIsEditing(false);
    setEditContent("");
  };

  const handleDownload = () => {
    if (!fileData?.content) return;

    const blob = new Blob([fileData.content], {
      type: fileData.encoding === "base64" ? "application/octet-stream" : "text/plain",
    });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = file.name;
    document.body.appendChild(a);
    a.click();
    document.body.removeChild(a);
    URL.revokeObjectURL(url);
  };

  const extension = getFileExtension(file.path);
  const isTextFile = fileData?.encoding === "text";
  const isBinary = fileData?.encoding === "base64";

  return (
    <Card className="h-full flex flex-col">
      <CardHeader className="pb-2 border-b">
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-2 min-w-0">
            <FileIcon extension={extension} />
            <CardTitle className="text-sm font-medium truncate">
              {file.name}
            </CardTitle>
            {file.is_readonly && (
              <Badge variant="outline" className="text-xs shrink-0">
                <Lock className="h-3 w-3 mr-1" />
                Read-only
              </Badge>
            )}
            <Badge variant="secondary" className="text-xs shrink-0">
              {formatFileSize(file.size_bytes)}
            </Badge>
          </div>
          <div className="flex items-center gap-1 shrink-0">
            {isTextFile && !file.is_readonly && !isEditing && (
              <Button
                variant="ghost"
                size="icon"
                className="h-7 w-7"
                onClick={handleEdit}
                title="Edit"
              >
                <Edit3 className="h-3.5 w-3.5" />
              </Button>
            )}
            {isEditing && (
              <>
                <Button
                  variant="ghost"
                  size="icon"
                  className="h-7 w-7"
                  onClick={handleCancel}
                  title="Cancel"
                >
                  <X className="h-3.5 w-3.5" />
                </Button>
                <Button
                  variant="ghost"
                  size="icon"
                  className="h-7 w-7"
                  onClick={handleSave}
                  disabled={updateFile.isPending}
                  title="Save"
                >
                  <Save className="h-3.5 w-3.5" />
                </Button>
              </>
            )}
            <Button
              variant="ghost"
              size="icon"
              className="h-7 w-7"
              onClick={handleDownload}
              title="Download"
            >
              <Download className="h-3.5 w-3.5" />
            </Button>
            <Button
              variant="ghost"
              size="icon"
              className="h-7 w-7"
              onClick={onClose}
              title="Close"
            >
              <X className="h-3.5 w-3.5" />
            </Button>
          </div>
        </div>
        <div className="text-xs text-muted-foreground truncate">
          {file.path}
        </div>
      </CardHeader>
      <CardContent className="flex-1 p-0 overflow-hidden">
        {isLoading ? (
          <div className="p-4 text-sm text-muted-foreground">Loading...</div>
        ) : isBinary ? (
          <div className="p-4 text-sm text-muted-foreground text-center">
            <File className="h-12 w-12 mx-auto mb-2 text-gray-300" />
            <p>Binary file</p>
            <p className="text-xs mt-1">Use download to view this file</p>
          </div>
        ) : isEditing ? (
          <textarea
            className="w-full h-full p-4 text-sm font-mono resize-none focus:outline-none"
            value={editContent}
            onChange={(e) => setEditContent(e.target.value)}
            spellCheck={false}
          />
        ) : (
          <ScrollArea className="h-full">
            <pre className="p-4 text-sm font-mono whitespace-pre-wrap break-words">
              {fileData?.content ?? "Empty file"}
            </pre>
          </ScrollArea>
        )}
      </CardContent>
    </Card>
  );
}

function FileIcon({ extension }: { extension: string }) {
  const isText = ["txt", "md", "json", "js", "ts", "tsx", "jsx", "css", "html", "py", "rs", "go", "yml", "yaml", "toml"].includes(extension.toLowerCase());

  if (isText) {
    return <FileText className="h-4 w-4 text-gray-500" />;
  }

  return <File className="h-4 w-4 text-gray-400" />;
}
