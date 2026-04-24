"use client";

/**
 * Shared initial_files preview used by agent and harness preview surfaces.
 * Keep rendering local to the UI because file content already exists client-side.
 */

import { useEffect, useMemo, useState } from "react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { ScrollArea } from "@/components/ui/scroll-area";
import { FilePreview, getPreviewType, type PreviewType } from "@/components/files/file-previews";
import type { InitialFile } from "@/lib/api/types";
import {
  File,
  FileArchive,
  FileCode,
  FileImage,
  FileJson,
  FileSpreadsheet,
  FileText,
  Lock,
} from "lucide-react";

interface InitialFilesPreviewProps {
  files: InitialFile[];
  title?: string;
  description?: string;
}

export function InitialFilesPreview({
  files,
  title = "Initial Files",
  description = "Files configured on this item. New sessions may merge files from other layers, with later layers overriding earlier files by path.",
}: InitialFilesPreviewProps) {
  const sortedFiles = useMemo(
    () => [...files].sort((left, right) => left.path.localeCompare(right.path)),
    [files],
  );
  const [selectedPath, setSelectedPath] = useState<string | null>(sortedFiles[0]?.path ?? null);

  useEffect(() => {
    if (sortedFiles.length === 0) {
      setSelectedPath(null);
      return;
    }

    if (!sortedFiles.some((file) => file.path === selectedPath)) {
      setSelectedPath(sortedFiles[0].path);
    }
  }, [selectedPath, sortedFiles]);

  const selectedFile =
    sortedFiles.find((file) => file.path === selectedPath) ?? sortedFiles[0] ?? null;

  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2">
          <FileText className="w-5 h-5" />
          {title}
          <Badge variant="secondary" className="ml-2">
            {sortedFiles.length}
          </Badge>
        </CardTitle>
        <CardDescription>{description}</CardDescription>
      </CardHeader>
      <CardContent>
        {sortedFiles.length === 0 ? (
          <p className="text-sm text-muted-foreground italic">No initial files configured.</p>
        ) : (
          <div className="grid gap-4 lg:grid-cols-[minmax(0,260px)_minmax(0,1fr)]">
            <ScrollArea className="h-[420px] border bg-muted/20">
              <div className="p-2 space-y-1">
                {sortedFiles.map((file) => {
                  const previewType = getInitialFilePreviewType(file);
                  const Icon = getPreviewIcon(previewType);
                  const isSelected = file.path === selectedFile?.path;

                  return (
                    <Button
                      key={file.path}
                      type="button"
                      variant={isSelected ? "secondary" : "ghost"}
                      className="h-auto w-full justify-start px-3 py-2"
                      onClick={() => setSelectedPath(file.path)}
                    >
                      <div className="flex min-w-0 flex-1 items-start gap-2 text-left">
                        <Icon className="mt-0.5 h-4 w-4 shrink-0 text-muted-foreground" />
                        <div className="min-w-0 space-y-1">
                          <div className="truncate font-mono text-xs">{file.path}</div>
                          <div className="flex flex-wrap gap-1">
                            <Badge variant="outline" className="text-[10px]">
                              {file.encoding}
                            </Badge>
                            {file.is_readonly && (
                              <Badge variant="outline" className="gap-1 text-[10px]">
                                <Lock className="h-3 w-3" />
                                read-only
                              </Badge>
                            )}
                          </div>
                        </div>
                      </div>
                    </Button>
                  );
                })}
              </div>
            </ScrollArea>

            <div className="border bg-muted/20">
              {selectedFile ? (
                <InitialFileContentPreview file={selectedFile} />
              ) : (
                <div className="flex h-[420px] items-center justify-center p-6 text-sm text-muted-foreground">
                  Select a file to preview its contents.
                </div>
              )}
            </div>
          </div>
        )}
      </CardContent>
    </Card>
  );
}

function InitialFileContentPreview({ file }: { file: InitialFile }) {
  const previewType = getInitialFilePreviewType(file);
  const extension = getFileExtension(file.path);

  if (previewType === "binary") {
    return (
      <div className="flex h-[420px] flex-col items-center justify-center gap-3 p-6 text-center">
        <FileArchive className="h-10 w-10 text-muted-foreground" />
        <div className="space-y-1">
          <p className="text-sm font-medium">Binary file</p>
          <p className="text-sm text-muted-foreground">
            This asset is stored as base64 and can be copied into new sessions, but it does not have
            an inline preview.
          </p>
        </div>
      </div>
    );
  }

  if (previewType === "text") {
    return (
      <ScrollArea className="h-[420px]">
        <pre className="min-h-full whitespace-pre-wrap break-words p-4 font-mono text-sm">
          {file.content}
        </pre>
      </ScrollArea>
    );
  }

  return (
    <div className="h-[420px]">
      <FilePreview content={file.content} extension={extension} encoding={file.encoding} />
    </div>
  );
}

function getInitialFilePreviewType(file: InitialFile): PreviewType {
  return getPreviewType(getFileExtension(file.path), file.encoding);
}

function getFileExtension(path: string): string {
  const normalizedPath = path.split("/").pop() ?? path;
  const extension = normalizedPath.includes(".") ? normalizedPath.split(".").pop() : "";
  return extension?.toLowerCase() ?? "";
}

function getPreviewIcon(type: PreviewType) {
  switch (type) {
    case "code":
      return FileCode;
    case "csv":
      return FileSpreadsheet;
    case "json":
      return FileJson;
    case "image":
    case "svg":
      return FileImage;
    case "markdown":
    case "text":
      return FileText;
    case "binary":
      return FileArchive;
    default:
      return File;
  }
}
