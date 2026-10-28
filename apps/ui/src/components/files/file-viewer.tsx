"use client";

import { useUiTranslations } from "@/providers/locale-provider";
import { relativeFilePath } from "@/lib/path-utils";
import { useState, type ReactNode } from "react";
import { useQuery } from "@tanstack/react-query";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { ScrollArea } from "@/components/ui/scroll-area";
import { File, X, Save, Edit3, Lock, Download, Eye, Code2 } from "lucide-react";
import { FileIcon } from "./file-icon";
import { FilePreview, canPreview, getPreviewType } from "./file-previews";
import { formatFileSize, getFileExtension } from "@/lib/api/session-files";
import type { FileInfo } from "@/lib/api/types";
import type { FileSource } from "./file-source";

interface FileViewerProps {
  /** The filesystem the file lives in; without `writes` it renders read-only. */
  source: FileSource;
  file: FileInfo;
  /** Omit to hide the close button, for viewers that always show a file. */
  onClose?: () => void;
  /** Badges after the file name, such as extraction state. */
  badges?: ReactNode;
  /** Actions before download, such as delete. */
  actions?: ReactNode;
  /** A strip under the content, such as per-file settings. */
  footer?: ReactNode;
}

type ViewMode = "preview" | "source";

export function FileViewer({ source, file, onClose, badges, actions, footer }: FileViewerProps) {
  const ui = useUiTranslations();
  const [isEditing, setIsEditing] = useState(false);
  const [editContent, setEditContent] = useState("");
  const [viewMode, setViewMode] = useState<ViewMode>("preview");

  const readOnly = !source.writes;
  const {
    data: fileData,
    isLoading,
    refetch,
  } = useQuery({
    queryKey: ["file-source", source.key, "file", file.path],
    queryFn: () => source.read(file.path),
    enabled: !!source.key,
  });
  const [saving, setSaving] = useState(false);

  const handleEdit = () => {
    if (fileData?.content) {
      setEditContent(fileData.content);
      setIsEditing(true);
    }
  };

  const handleSave = async () => {
    if (!source.writes) return;
    setSaving(true);
    try {
      await source.writes.updateFile(file.path, editContent);
      setIsEditing(false);
      refetch();
    } catch {
      // The editor stays open with the unsaved text.
    } finally {
      setSaving(false);
    }
  };

  const handleCancel = () => {
    setIsEditing(false);
    setEditContent("");
  };

  const handleDownload = () => {
    // The raw download gives the real bytes: binary content arrives
    // base64-encoded, and some sources only show a text view of the original.
    const rawUrl = source.downloadUrl(file.path);
    if (rawUrl && (fileData?.encoding === "base64" || !source.writes)) {
      const a = document.createElement("a");
      a.href = rawUrl;
      a.download = file.name;
      document.body.appendChild(a);
      a.click();
      document.body.removeChild(a);
      return;
    }
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
  const encoding = (fileData?.encoding ?? "text") as "text" | "base64";
  const hasPreview = canPreview(extension, encoding);
  // HTML previews load through the sandboxed server endpoint so their JS can run
  // in an isolated origin; only available once we know the workspace.
  const previewType = getPreviewType(extension, encoding);
  const htmlPreviewSrc = previewType === "html" ? source.htmlPreviewUrl?.(file.path) : undefined;
  // Videos stream from the raw download so the player can seek.
  const videoSrc = previewType === "video" ? source.downloadUrl(file.path) : undefined;

  return (
    <Card className="h-full flex flex-col">
      <CardHeader className="pb-2 border-b">
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-2 min-w-0">
            <FileIcon extension={extension} />
            <CardTitle className="text-sm font-medium truncate">{file.name}</CardTitle>
            {file.is_readonly && !readOnly && (
              <Badge variant="outline" className="text-xs shrink-0">
                <Lock className="h-3 w-3 mr-1" />
                {ui("Read-only")}
              </Badge>
            )}
            <Badge variant="secondary" className="text-xs shrink-0">
              {formatFileSize(file.size_bytes)}
            </Badge>
            {badges}
          </div>
          <div className="flex items-center gap-1 shrink-0">
            {hasPreview && !isEditing && (
              <div className="flex items-center border mr-1">
                <Button
                  variant={viewMode === "preview" ? "secondary" : "ghost"}
                  size="sm"
                  className="h-7 px-2"
                  onClick={() => setViewMode("preview")}
                  title={ui("View")}
                >
                  <Eye className="h-3.5 w-3.5 mr-1" />
                  <span className="text-xs">{ui("View")}</span>
                </Button>
                <Button
                  variant={viewMode === "source" ? "secondary" : "ghost"}
                  size="sm"
                  className="h-7 px-2"
                  onClick={() => setViewMode("source")}
                  title={ui("Source")}
                >
                  <Code2 className="h-3.5 w-3.5 mr-1" />
                  <span className="text-xs">{ui("Source")}</span>
                </Button>
              </div>
            )}
            {!readOnly && isTextFile && !file.is_readonly && !isEditing && (
              <Button
                variant="ghost"
                size="icon"
                className="h-7 w-7"
                onClick={handleEdit}
                title={ui("Edit")}
              >
                <Edit3 className="h-3.5 w-3.5" />
              </Button>
            )}
            {!readOnly && isEditing && (
              <>
                <Button
                  variant="ghost"
                  size="icon"
                  className="h-7 w-7"
                  onClick={handleCancel}
                  title={ui("Cancel")}
                >
                  <X className="h-3.5 w-3.5" />
                </Button>
                <Button
                  variant="ghost"
                  size="icon"
                  className="h-7 w-7"
                  onClick={handleSave}
                  disabled={saving}
                  title={ui("Save")}
                >
                  <Save className="h-3.5 w-3.5" />
                </Button>
              </>
            )}
            {actions}
            <Button
              variant="ghost"
              size="icon"
              className="h-7 w-7"
              onClick={handleDownload}
              title={ui("Download")}
            >
              <Download className="h-3.5 w-3.5" />
            </Button>
            {onClose && (
              <Button
                variant="ghost"
                size="icon"
                className="h-7 w-7"
                onClick={onClose}
                title={ui("Close")}
              >
                <X className="h-3.5 w-3.5" />
              </Button>
            )}
          </div>
        </div>
        <div className="text-xs text-muted-foreground truncate">{relativeFilePath(file.path)}</div>
      </CardHeader>
      <CardContent className="flex-1 p-0 overflow-hidden">
        {isLoading ? (
          <div className="p-4 text-sm text-muted-foreground">{ui("Loading...")}</div>
        ) : isBinary && !hasPreview ? (
          <div className="p-4 text-sm text-muted-foreground text-center">
            <File className="h-12 w-12 mx-auto mb-2 text-muted-foreground/60" />
            <p>{ui("Binary file")}</p>
            <p className="text-xs mt-1">{ui("Use download to view this file")}</p>
          </div>
        ) : !readOnly && isEditing ? (
          <textarea
            className="w-full h-full p-4 text-sm font-mono resize-none focus:outline-none"
            value={editContent}
            onChange={(e) => setEditContent(e.target.value)}
            spellCheck={false}
            aria-label={ui("File content editor")}
          />
        ) : !fileData?.content && fileData?.content !== "" ? (
          <div className="flex flex-col items-center justify-center h-full text-muted-foreground">
            <File className="h-12 w-12 mb-2 opacity-30" />
            <p className="text-sm">{ui("Empty file")}</p>
          </div>
        ) : hasPreview && viewMode === "preview" ? (
          <FilePreview
            content={fileData.content}
            extension={extension}
            encoding={encoding}
            htmlPreviewSrc={htmlPreviewSrc}
            videoSrc={videoSrc}
          />
        ) : (
          <ScrollArea className="h-full">
            <pre className="p-4 text-sm font-mono whitespace-pre-wrap break-words">
              {fileData?.content || ui("Empty file")}
            </pre>
          </ScrollArea>
        )}
      </CardContent>
      {footer}
    </Card>
  );
}

// FileIcon is now imported from ./file-icon.tsx
