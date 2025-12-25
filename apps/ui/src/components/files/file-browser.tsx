"use client";

import { useState } from "react";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Badge } from "@/components/ui/badge";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  Folder,
  File,
  ChevronRight,
  ChevronDown,
  Plus,
  FolderPlus,
  Trash2,
  RefreshCw,
  Home,
  FileText,
  Lock,
} from "lucide-react";
import { useFiles, useCreateFile, useCreateDirectory, useDeleteFile } from "@/hooks/use-session-files";
import { formatFileSize, getParentPath, joinPath } from "@/lib/api/session-files";
import type { FileInfo } from "@/lib/api/types";

interface FileBrowserProps {
  agentId: string;
  sessionId: string;
  onFileSelect?: (file: FileInfo) => void;
  selectedPath?: string;
}

export function FileBrowser({
  agentId,
  sessionId,
  onFileSelect,
  selectedPath,
}: FileBrowserProps) {
  const [currentPath, setCurrentPath] = useState("/");
  const [expandedDirs, setExpandedDirs] = useState<Set<string>>(new Set(["/"]));
  const [isCreateFileOpen, setIsCreateFileOpen] = useState(false);
  const [isCreateDirOpen, setIsCreateDirOpen] = useState(false);
  const [newFileName, setNewFileName] = useState("");
  const [newDirName, setNewDirName] = useState("");
  const [newFileContent, setNewFileContent] = useState("");

  const { data: files, isLoading, refetch } = useFiles(agentId, sessionId, currentPath, false);
  const createFile = useCreateFile();
  const createDir = useCreateDirectory();
  const deleteFile = useDeleteFile();

  const toggleDir = (path: string) => {
    const newExpanded = new Set(expandedDirs);
    if (newExpanded.has(path)) {
      newExpanded.delete(path);
    } else {
      newExpanded.add(path);
    }
    setExpandedDirs(newExpanded);
  };

  const handleFileClick = (file: FileInfo) => {
    if (file.is_directory) {
      setCurrentPath(file.path);
      toggleDir(file.path);
    } else {
      onFileSelect?.(file);
    }
  };

  const navigateUp = () => {
    const parent = getParentPath(currentPath);
    if (parent) {
      setCurrentPath(parent);
    }
  };

  const navigateHome = () => {
    setCurrentPath("/");
  };

  const handleCreateFile = async () => {
    if (!newFileName.trim()) return;

    try {
      await createFile.mutateAsync({
        agentId,
        sessionId,
        request: {
          path: joinPath(currentPath, newFileName),
          content: newFileContent,
          encoding: "text",
        },
      });
      setIsCreateFileOpen(false);
      setNewFileName("");
      setNewFileContent("");
      refetch();
    } catch {
      // Error handling done by react-query
    }
  };

  const handleCreateDir = async () => {
    if (!newDirName.trim()) return;

    try {
      await createDir.mutateAsync({
        agentId,
        sessionId,
        path: joinPath(currentPath, newDirName),
      });
      setIsCreateDirOpen(false);
      setNewDirName("");
      refetch();
    } catch {
      // Error handling done by react-query
    }
  };

  const handleDelete = async (file: FileInfo) => {
    if (!confirm(`Delete ${file.is_directory ? "directory" : "file"} "${file.name}"?`)) {
      return;
    }

    try {
      await deleteFile.mutateAsync({
        agentId,
        sessionId,
        path: file.path,
        recursive: file.is_directory,
      });
      refetch();
    } catch {
      // Error handling done by react-query
    }
  };

  const breadcrumbs = currentPath.split("/").filter(Boolean);

  return (
    <Card className="h-full flex flex-col">
      <CardHeader className="pb-2">
        <div className="flex items-center justify-between">
          <CardTitle className="text-sm font-medium flex items-center gap-2">
            <Folder className="h-4 w-4" />
            Files
          </CardTitle>
          <div className="flex items-center gap-1">
            <Button
              variant="ghost"
              size="icon"
              className="h-7 w-7"
              onClick={() => refetch()}
              title="Refresh"
            >
              <RefreshCw className="h-3.5 w-3.5" />
            </Button>
            <Button
              variant="ghost"
              size="icon"
              className="h-7 w-7"
              title="New folder"
              onClick={() => setIsCreateDirOpen(true)}
            >
              <FolderPlus className="h-3.5 w-3.5" />
            </Button>
            <Dialog open={isCreateDirOpen} onOpenChange={setIsCreateDirOpen}>
              <DialogContent>
                <DialogHeader>
                  <DialogTitle>Create Folder</DialogTitle>
                  <DialogDescription>
                    Create a new folder in {currentPath}
                  </DialogDescription>
                </DialogHeader>
                <Input
                  placeholder="Folder name"
                  value={newDirName}
                  onChange={(e) => setNewDirName(e.target.value)}
                  onKeyDown={(e) => e.key === "Enter" && handleCreateDir()}
                />
                <DialogFooter>
                  <Button variant="outline" onClick={() => setIsCreateDirOpen(false)}>
                    Cancel
                  </Button>
                  <Button onClick={handleCreateDir} disabled={!newDirName.trim()}>
                    Create
                  </Button>
                </DialogFooter>
              </DialogContent>
            </Dialog>
            <Button
              variant="ghost"
              size="icon"
              className="h-7 w-7"
              title="New file"
              onClick={() => setIsCreateFileOpen(true)}
            >
              <Plus className="h-3.5 w-3.5" />
            </Button>
            <Dialog open={isCreateFileOpen} onOpenChange={setIsCreateFileOpen}>
              <DialogContent>
                <DialogHeader>
                  <DialogTitle>Create File</DialogTitle>
                  <DialogDescription>
                    Create a new file in {currentPath}
                  </DialogDescription>
                </DialogHeader>
                <div className="space-y-4">
                  <Input
                    placeholder="File name"
                    value={newFileName}
                    onChange={(e) => setNewFileName(e.target.value)}
                  />
                  <textarea
                    className="w-full h-32 p-2 text-sm border rounded-md font-mono"
                    placeholder="File content (optional)"
                    value={newFileContent}
                    onChange={(e) => setNewFileContent(e.target.value)}
                  />
                </div>
                <DialogFooter>
                  <Button variant="outline" onClick={() => setIsCreateFileOpen(false)}>
                    Cancel
                  </Button>
                  <Button onClick={handleCreateFile} disabled={!newFileName.trim()}>
                    Create
                  </Button>
                </DialogFooter>
              </DialogContent>
            </Dialog>
          </div>
        </div>
        {/* Breadcrumbs */}
        <div className="flex items-center gap-1 text-xs text-muted-foreground overflow-x-auto">
          <Button
            variant="ghost"
            size="icon"
            className="h-5 w-5 shrink-0"
            onClick={navigateHome}
          >
            <Home className="h-3 w-3" />
          </Button>
          {breadcrumbs.map((crumb, index) => (
            <span key={crumb} className="flex items-center">
              <ChevronRight className="h-3 w-3 mx-0.5" />
              <button
                type="button"
                className="hover:text-foreground hover:underline"
                onClick={() => {
                  const path = `/${breadcrumbs.slice(0, index + 1).join("/")}`;
                  setCurrentPath(path);
                }}
              >
                {crumb}
              </button>
            </span>
          ))}
        </div>
      </CardHeader>
      <CardContent className="flex-1 p-0 overflow-hidden">
        <ScrollArea className="h-full">
          {isLoading ? (
            <div className="p-4 text-sm text-muted-foreground">Loading...</div>
          ) : !files?.length ? (
            <div className="p-4 text-sm text-muted-foreground text-center">
              Empty folder
            </div>
          ) : (
            <div className="p-2">
              {/* Show parent directory link if not at root */}
              {currentPath !== "/" && (
                <button
                  type="button"
                  className="w-full flex items-center gap-2 p-2 text-sm text-muted-foreground hover:bg-muted rounded-md"
                  onClick={navigateUp}
                >
                  <Folder className="h-4 w-4" />
                  <span>..</span>
                </button>
              )}
              {files.map((file) => (
                <FileItem
                  key={file.id}
                  file={file}
                  isSelected={selectedPath === file.path}
                  isExpanded={expandedDirs.has(file.path)}
                  onClick={() => handleFileClick(file)}
                  onDelete={() => handleDelete(file)}
                />
              ))}
            </div>
          )}
        </ScrollArea>
      </CardContent>
    </Card>
  );
}

interface FileItemProps {
  file: FileInfo;
  isSelected: boolean;
  isExpanded: boolean;
  onClick: () => void;
  onDelete: () => void;
}

function FileItem({ file, isSelected, isExpanded, onClick, onDelete }: FileItemProps) {
  const [showActions, setShowActions] = useState(false);

  return (
    <div
      className={`group flex items-center gap-2 p-2 text-sm rounded-md cursor-pointer transition-colors ${
        isSelected ? "bg-accent text-accent-foreground" : "hover:bg-muted"
      }`}
      onClick={onClick}
      onKeyDown={(e) => e.key === "Enter" && onClick()}
      onMouseEnter={() => setShowActions(true)}
      onMouseLeave={() => setShowActions(false)}
    >
      {file.is_directory ? (
        <>
          {isExpanded ? (
            <ChevronDown className="h-4 w-4 text-muted-foreground" />
          ) : (
            <ChevronRight className="h-4 w-4 text-muted-foreground" />
          )}
          <Folder className="h-4 w-4 text-blue-500" />
        </>
      ) : (
        <>
          <span className="w-4" />
          <FileIcon extension={file.name.split(".").pop() ?? ""} />
        </>
      )}
      <span className="flex-1 truncate">{file.name}</span>
      <div className="flex items-center gap-1">
        {file.is_readonly && (
          <Lock className="h-3 w-3 text-muted-foreground" />
        )}
        {!file.is_directory && (
          <Badge variant="outline" className="text-xs py-0">
            {formatFileSize(file.size_bytes)}
          </Badge>
        )}
        {showActions && (
          <Button
            variant="ghost"
            size="icon"
            className="h-6 w-6 opacity-0 group-hover:opacity-100"
            onClick={(e) => {
              e.stopPropagation();
              onDelete();
            }}
          >
            <Trash2 className="h-3 w-3 text-destructive" />
          </Button>
        )}
      </div>
    </div>
  );
}

function FileIcon({ extension }: { extension: string }) {
  const isText = ["txt", "md", "json", "js", "ts", "tsx", "jsx", "css", "html", "py", "rs", "go", "yml", "yaml", "toml"].includes(extension.toLowerCase());

  if (isText) {
    return <FileText className="h-4 w-4 text-gray-500" />;
  }

  return <File className="h-4 w-4 text-gray-400" />;
}
