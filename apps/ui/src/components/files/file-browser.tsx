"use client";

import { useState, useCallback, useEffect } from "react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { ScrollArea } from "@/components/ui/scroll-area";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  Plus,
  FolderPlus,
  Trash2,
  RefreshCw,
  Lock,
  FileCode,
  FileText,
  FileJson,
  Image,
  File,
  Loader2,
} from "lucide-react";
import {
  FileTree,
  FileTreeFolder,
  FileTreeFile,
  FileTreeActions,
  FileTreeName,
} from "@/components/ai-elements/file-tree";
import {
  useFiles,
  useCreateFile,
  useCreateDirectory,
  useDeleteFile,
} from "@/hooks/use-session-files";
import { formatFileSize, joinPath } from "@/lib/api/session-files";
import type { FileInfo } from "@/lib/api/types";

interface FileBrowserProps {
  sessionId: string;
  onFileSelect?: (file: FileInfo) => void;
  selectedPath?: string;
}

// Get file icon based on extension with muted colors matching app theme
function getFileIcon(filename: string) {
  const ext = filename.split(".").pop()?.toLowerCase();
  switch (ext) {
    case "ts":
    case "tsx":
    case "js":
    case "jsx":
    case "py":
    case "rs":
    case "go":
    case "java":
    case "c":
    case "cpp":
    case "h":
    case "hpp":
    case "rb":
    case "php":
    case "swift":
    case "kt":
    case "scala":
    case "sh":
    case "bash":
    case "zsh":
    case "css":
    case "scss":
    case "less":
    case "html":
    case "vue":
    case "svelte":
      return <FileCode className="size-4 text-sky-600 dark:text-sky-400" />;
    case "json":
    case "yaml":
    case "yml":
    case "toml":
    case "xml":
      return <FileJson className="size-4 text-amber-600 dark:text-amber-400" />;
    case "md":
    case "mdx":
    case "txt":
    case "csv":
      return <FileText className="size-4 text-muted-foreground" />;
    case "png":
    case "jpg":
    case "jpeg":
    case "gif":
    case "svg":
    case "webp":
    case "ico":
      return <Image className="size-4 text-violet-600 dark:text-violet-400" />;
    default:
      return <File className="size-4 text-muted-foreground" />;
  }
}

export function FileBrowser({ sessionId, onFileSelect, selectedPath }: FileBrowserProps) {
  const [expandedPaths, setExpandedPaths] = useState<Set<string>>(new Set(["/workspace"]));
  const [loadedDirs, setLoadedDirs] = useState<Map<string, FileInfo[]>>(new Map());
  const [isCreateFileOpen, setIsCreateFileOpen] = useState(false);
  const [isCreateDirOpen, setIsCreateDirOpen] = useState(false);
  const [newFileName, setNewFileName] = useState("");
  const [newDirName, setNewDirName] = useState("");
  const [newFileContent, setNewFileContent] = useState("");
  const [createPath, setCreatePath] = useState("/workspace");

  // Load root directory
  const {
    data: rootFiles,
    isLoading,
    refetch: refetchRoot,
  } = useFiles(sessionId, "/workspace", false);
  const createFile = useCreateFile();
  const createDir = useCreateDirectory();
  const deleteFile = useDeleteFile();

  // Update loaded dirs when root files change
  useEffect(() => {
    if (rootFiles) {
      setLoadedDirs((prev) => new Map(prev).set("/workspace", rootFiles));
    }
  }, [rootFiles]);

  const handleSelect = useCallback(
    (path: string) => {
      // Find the file info for this path
      for (const files of loadedDirs.values()) {
        const file = files.find((f) => f.path === path);
        if (file) {
          if (!file.is_directory) {
            onFileSelect?.(file);
          }
          return;
        }
      }
    },
    [loadedDirs, onFileSelect],
  );

  // Function to load a directory's contents
  const loadDirectory = useCallback(
    async (path: string) => {
      try {
        const response = await fetch(
          `/api/sessions/${sessionId}/files?path=${encodeURIComponent(path)}`,
        );
        if (response.ok) {
          const data = await response.json();
          setLoadedDirs((prev) => new Map(prev).set(path, data.data || data));
        }
      } catch {
        // Error loading directory
      }
    },
    [sessionId],
  );

  const handleExpandedChange = useCallback(
    (newExpanded: Set<string>) => {
      setExpandedPaths(newExpanded);

      // Load any newly expanded directories that haven't been loaded
      for (const path of newExpanded) {
        if (!loadedDirs.has(path) && path !== "/workspace") {
          loadDirectory(path);
        }
      }
    },
    [loadedDirs, loadDirectory],
  );

  const handleRefresh = useCallback(() => {
    setLoadedDirs(new Map());
    refetchRoot();
  }, [refetchRoot]);

  const handleCreateFile = async () => {
    if (!newFileName.trim()) return;

    try {
      await createFile.mutateAsync({
        sessionId,
        request: {
          path: joinPath(createPath, newFileName),
          content: newFileContent,
          encoding: "text",
        },
      });
      setIsCreateFileOpen(false);
      setNewFileName("");
      setNewFileContent("");
      handleRefresh();
    } catch {
      // Error handling done by react-query
    }
  };

  const handleCreateDir = async () => {
    if (!newDirName.trim()) return;

    try {
      await createDir.mutateAsync({
        sessionId,
        path: joinPath(createPath, newDirName),
      });
      setIsCreateDirOpen(false);
      setNewDirName("");
      handleRefresh();
    } catch {
      // Error handling done by react-query
    }
  };

  const handleDelete = async (file: FileInfo, e: React.MouseEvent) => {
    e.stopPropagation();
    if (!confirm(`Delete ${file.is_directory ? "directory" : "file"} "${file.name}"?`)) {
      return;
    }

    try {
      await deleteFile.mutateAsync({
        sessionId,
        path: file.path,
        recursive: file.is_directory,
      });
      handleRefresh();
    } catch {
      // Error handling done by react-query
    }
  };

  const openCreateFileDialog = (path: string) => {
    setCreatePath(path);
    setIsCreateFileOpen(true);
  };

  const openCreateDirDialog = (path: string) => {
    setCreatePath(path);
    setIsCreateDirOpen(true);
  };

  // Recursive component to render directory contents
  const renderDirectoryContents = (path: string) => {
    const files = loadedDirs.get(path);
    if (!files) {
      return null;
    }

    // Sort: directories first, then files, both alphabetically
    const sortedFiles = [...files].sort((a, b) => {
      if (a.is_directory !== b.is_directory) {
        return a.is_directory ? -1 : 1;
      }
      return a.name.localeCompare(b.name);
    });

    return sortedFiles.map((file) => {
      if (file.is_directory) {
        return (
          <FileTreeFolder key={file.id} path={file.path} name={file.name}>
            <FileTreeActions>
              {file.is_readonly && <Lock className="size-3 text-muted-foreground" />}
              <Button
                variant="ghost"
                size="icon"
                className="size-5"
                onClick={(e) => {
                  e.stopPropagation();
                  openCreateFileDialog(file.path);
                }}
                title="New file"
              >
                <Plus className="size-3" />
              </Button>
              <Button
                variant="ghost"
                size="icon"
                className="size-5"
                onClick={(e) => {
                  e.stopPropagation();
                  openCreateDirDialog(file.path);
                }}
                title="New folder"
              >
                <FolderPlus className="size-3" />
              </Button>
              {!file.is_readonly && (
                <Button
                  variant="ghost"
                  size="icon"
                  className="size-5"
                  onClick={(e) => handleDelete(file, e)}
                  title="Delete"
                >
                  <Trash2 className="size-3 text-destructive" />
                </Button>
              )}
            </FileTreeActions>
            {renderDirectoryContents(file.path)}
          </FileTreeFolder>
        );
      }

      return (
        <FileTreeFile key={file.id} path={file.path} name={file.name} icon={getFileIcon(file.name)}>
          {/* Spacer for alignment with folder chevrons */}
          <span className="size-3.5" />
          {getFileIcon(file.name)}
          <FileTreeName className="flex-1">{file.name}</FileTreeName>
          <FileTreeActions>
            {file.is_readonly && <Lock className="size-3 text-muted-foreground" />}
            <span className="text-xs text-muted-foreground tabular-nums">
              {formatFileSize(file.size_bytes)}
            </span>
            {!file.is_readonly && (
              <Button
                variant="ghost"
                size="icon"
                className="size-5"
                onClick={(e) => handleDelete(file, e)}
                title="Delete"
              >
                <Trash2 className="size-3 text-destructive" />
              </Button>
            )}
          </FileTreeActions>
        </FileTreeFile>
      );
    });
  };

  return (
    <div className="h-full flex flex-col">
      {/* Toolbar */}
      <div className="flex items-center justify-end gap-1 px-2 py-1.5 border-b bg-muted/30">
        <Button
          variant="ghost"
          size="sm"
          className="h-7 px-2 gap-1.5"
          onClick={handleRefresh}
          title="Refresh"
        >
          <RefreshCw className="size-3.5" />
          <span className="text-xs">Refresh</span>
        </Button>
        <Button
          variant="ghost"
          size="sm"
          className="h-7 px-2 gap-1.5"
          onClick={() => openCreateDirDialog("/workspace")}
          title="New folder"
        >
          <FolderPlus className="size-3.5" />
          <span className="text-xs">Folder</span>
        </Button>
        <Button
          variant="ghost"
          size="sm"
          className="h-7 px-2 gap-1.5"
          onClick={() => openCreateFileDialog("/workspace")}
          title="New file"
        >
          <Plus className="size-3.5" />
          <span className="text-xs">File</span>
        </Button>
      </div>

      {/* File Tree */}
      <ScrollArea className="flex-1">
        {isLoading ? (
          <div className="flex items-center justify-center py-8 text-muted-foreground">
            <Loader2 className="size-4 animate-spin mr-2" />
            <span className="text-sm">Loading files...</span>
          </div>
        ) : !rootFiles?.length ? (
          <div className="flex flex-col items-center justify-center py-12 text-muted-foreground">
            <File className="size-8 mb-2 opacity-40" />
            <p className="text-sm">Empty workspace</p>
            <p className="text-xs mt-1">Create a file or folder to get started</p>
          </div>
        ) : (
          <FileTree
            expanded={expandedPaths}
            selectedPath={selectedPath}
            onSelect={handleSelect}
            onExpandedChange={handleExpandedChange}
          >
            {renderDirectoryContents("/workspace")}
          </FileTree>
        )}
      </ScrollArea>

      {/* Create File Dialog */}
      <Dialog open={isCreateFileOpen} onOpenChange={setIsCreateFileOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Create File</DialogTitle>
            <DialogDescription>
              Create a new file in{" "}
              <code className="text-xs bg-muted px-1 py-0.5">{createPath}</code>
            </DialogDescription>
          </DialogHeader>
          <div className="space-y-4">
            <Input
              placeholder="filename.txt"
              value={newFileName}
              onChange={(e) => setNewFileName(e.target.value)}
              onKeyDown={(e) => e.key === "Enter" && handleCreateFile()}
              className="font-mono"
            />
            <textarea
              className="w-full h-32 p-2 text-sm border font-mono bg-background resize-none"
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

      {/* Create Directory Dialog */}
      <Dialog open={isCreateDirOpen} onOpenChange={setIsCreateDirOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Create Folder</DialogTitle>
            <DialogDescription>
              Create a new folder in{" "}
              <code className="text-xs bg-muted px-1 py-0.5">{createPath}</code>
            </DialogDescription>
          </DialogHeader>
          <Input
            placeholder="folder-name"
            value={newDirName}
            onChange={(e) => setNewDirName(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && handleCreateDir()}
            className="font-mono"
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
    </div>
  );
}
