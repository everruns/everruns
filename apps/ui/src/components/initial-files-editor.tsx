"use client";

/**
 * Model initial_files as an in-memory filesystem so agent and harness forms can
 * reuse the session file browser/viewer patterns without requiring a session.
 */

import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import {
  FileTree,
  FileTreeActions,
  FileTreeFile,
  FileTreeFolder,
  FileTreeName,
} from "@/components/ai-elements/file-tree";
import {
  FilePreview,
  canPreview,
  getPreviewType,
  type PreviewType,
} from "@/components/files/file-previews";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Textarea } from "@/components/ui/textarea";
import { formatFileSize } from "@/lib/api/session-files";
import type { InitialFile } from "@/lib/api/types";
import {
  Eye,
  File,
  FileArchive,
  FileCode,
  FileImage,
  FileJson,
  FileSpreadsheet,
  FileText,
  Lock,
  PencilLine,
  Plus,
  Trash2,
  Upload,
} from "lucide-react";

interface InitialFilesEditorProps {
  value: InitialFile[];
  onChange: (files: InitialFile[]) => void;
  disabled?: boolean;
  title?: string;
  description?: string;
}

type TreeNode = TreeDirectoryNode | TreeFileNode;

interface TreeDirectoryNode {
  kind: "directory";
  id: string;
  name: string;
  children: TreeNode[];
}

interface TreeFileNode {
  kind: "file";
  id: string;
  index: number;
  name: string;
}

const EMPTY_TEXT_FILE: InitialFile = {
  path: "/new-file.txt",
  content: "",
  encoding: "text",
  is_readonly: false,
};

export function InitialFilesEditor({
  value,
  onChange,
  disabled,
  title = "Starter Files",
  description = "Files copied into each new session. Add text files directly or upload binary assets.",
}: InitialFilesEditorProps) {
  const files = useMemo(() => value ?? [], [value]);
  const uploadInputRef = useRef<HTMLInputElement>(null);
  const replaceInputRef = useRef<HTMLInputElement>(null);
  const [selectedIndex, setSelectedIndex] = useState<number | null>(files[0] ? 0 : null);
  const [expandedPaths, setExpandedPaths] = useState<Set<string>>(new Set());
  const [activeView, setActiveView] = useState<"source" | "preview">("source");

  const tree = useMemo(() => buildTree(files), [files]);
  const selectedFile = selectedIndex !== null ? (files[selectedIndex] ?? null) : null;
  const selectedFileId = selectedIndex !== null ? getFileId(selectedIndex) : undefined;
  const selectedPreviewType = selectedFile ? getInitialFilePreviewType(selectedFile) : null;
  const selectedCanPreview =
    selectedFile != null && canPreview(getFileExtension(selectedFile.path), selectedFile.encoding);

  useEffect(() => {
    if (files.length === 0) {
      setSelectedIndex(null);
      return;
    }

    if (selectedIndex === null || selectedIndex >= files.length) {
      setSelectedIndex(0);
    }
  }, [files, selectedIndex]);

  useEffect(() => {
    if (!selectedCanPreview && activeView === "preview") {
      setActiveView("source");
    }
  }, [activeView, selectedCanPreview]);

  useEffect(() => {
    setExpandedPaths((previous) => {
      if (tree.rootDirectoryIds.length === 0) {
        return previous;
      }

      const next = new Set(previous);
      for (const id of tree.rootDirectoryIds) {
        next.add(id);
      }
      return next;
    });
  }, [tree.rootDirectoryIds]);

  const updateFile = (index: number, patch: Partial<InitialFile>) => {
    const next = [...files];
    next[index] = { ...next[index], ...patch };
    onChange(next);
  };

  const removeFile = (index: number) => {
    const next = files.filter((_, current) => current !== index);
    onChange(next);

    if (next.length === 0) {
      setSelectedIndex(null);
      return;
    }

    if (selectedIndex === null) {
      setSelectedIndex(0);
      return;
    }

    if (index < selectedIndex) {
      setSelectedIndex(selectedIndex - 1);
      return;
    }

    if (index === selectedIndex) {
      setSelectedIndex(Math.min(index, next.length - 1));
    }
  };

  const addTextFile = () => {
    const next = [...files, { ...EMPTY_TEXT_FILE, path: `/file-${files.length + 1}.txt` }];
    onChange(next);
    setSelectedIndex(next.length - 1);
    setActiveView("source");
  };

  const addUploadedFiles = async (selected: FileList | null) => {
    if (!selected?.length) return;

    const uploaded = await Promise.all(
      Array.from(selected).map(async (file) => {
        const encoded = await encodeFile(file);
        return {
          path: `/${file.name}`,
          content: encoded.content,
          encoding: encoded.encoding,
          is_readonly: false,
        } satisfies InitialFile;
      }),
    );

    const next = [...files, ...uploaded];
    onChange(next);
    setSelectedIndex(files.length);
    setActiveView("source");
  };

  const replaceWithUpload = async (index: number, selected: FileList | null) => {
    if (!selected?.length) return;

    const file = selected[0];
    const encoded = await encodeFile(file);
    updateFile(index, {
      path: files[index]?.path || `/${file.name}`,
      content: encoded.content,
      encoding: encoded.encoding,
    });
    setActiveView("source");
  };

  const handleTreeSelect = (nodeId: string) => {
    if (!nodeId.startsWith("file:")) {
      return;
    }

    const index = Number(nodeId.slice("file:".length));
    if (Number.isNaN(index) || !files[index]) {
      return;
    }

    setSelectedIndex(index);
  };

  return (
    <div className="space-y-4">
      <div className="space-y-1">
        <Label>{title}</Label>
        <p className="text-xs text-muted-foreground">{description}</p>
      </div>

      <div className="flex flex-wrap items-center gap-2">
        <Button type="button" variant="outline" size="sm" onClick={addTextFile} disabled={disabled}>
          <Plus className="mr-2 h-4 w-4" />
          Add Text File
        </Button>
        <Button
          type="button"
          variant="outline"
          size="sm"
          onClick={() => uploadInputRef.current?.click()}
          disabled={disabled}
        >
          <Upload className="mr-2 h-4 w-4" />
          Upload File
        </Button>
        <Badge variant="secondary">{files.length} files</Badge>
        <input
          ref={uploadInputRef}
          type="file"
          multiple
          className="hidden"
          aria-label="Upload files"
          onChange={(event) => {
            void addUploadedFiles(event.target.files);
            event.target.value = "";
          }}
        />
      </div>

      {files.length === 0 ? (
        <div className="border border-dashed p-6 text-sm text-muted-foreground">
          No starter files configured. Add a text file or upload an asset to seed new sessions.
        </div>
      ) : (
        <div className="grid gap-4 lg:grid-cols-[minmax(0,300px)_minmax(0,1fr)]">
          <div className="flex min-h-[520px] flex-col border bg-muted/20">
            <div className="border-b px-3 py-2 text-xs text-muted-foreground">Files</div>
            <ScrollArea className="flex-1 min-h-0">
              <FileTree
                expanded={expandedPaths}
                selectedPath={selectedFileId}
                onExpandedChange={setExpandedPaths}
                onSelect={handleTreeSelect}
              >
                {tree.nodes.map((node) => renderTreeNode(node, files, removeFile, disabled))}
              </FileTree>
            </ScrollArea>
          </div>

          <div className="flex min-h-[520px] flex-col border bg-muted/20">
            {selectedFile ? (
              <>
                <div className="flex flex-wrap items-center justify-between gap-3 border-b px-4 py-3">
                  <div className="flex min-w-0 items-center gap-2">
                    <div className="shrink-0">{getPreviewIcon(selectedPreviewType)}</div>
                    <div className="min-w-0">
                      <div className="truncate font-medium">
                        {getDisplayName(selectedFile.path)}
                      </div>
                      <div className="truncate text-xs text-muted-foreground">
                        {selectedFile.path}
                      </div>
                    </div>
                  </div>
                  <div className="flex flex-wrap items-center gap-2">
                    <Badge variant="outline">{selectedFile.encoding}</Badge>
                    {selectedFile.is_readonly && (
                      <Badge variant="outline" className="gap-1">
                        <Lock className="h-3 w-3" />
                        read-only
                      </Badge>
                    )}
                    <Badge variant="secondary">
                      {formatFileSize(getFileSizeBytes(selectedFile))}
                    </Badge>
                  </div>
                </div>

                <div className="space-y-4 p-4">
                  <div className="space-y-2">
                    <Label htmlFor="initial-file-path">Path</Label>
                    <Input
                      id="initial-file-path"
                      value={selectedFile.path}
                      onChange={(event) => updateFile(selectedIndex!, { path: event.target.value })}
                      placeholder="/.agents/skills/my-skill/SKILL.md"
                      disabled={disabled}
                    />
                    <p className="text-xs text-muted-foreground">
                      Absolute path inside the session workspace. `/workspace/...` is also accepted.
                    </p>
                  </div>

                  <div className="flex flex-wrap items-center justify-between gap-3">
                    <div className="flex items-center gap-2">
                      <Checkbox
                        id="initial-file-readonly"
                        checked={selectedFile.is_readonly}
                        onCheckedChange={(checked) =>
                          updateFile(selectedIndex!, { is_readonly: checked === true })
                        }
                        disabled={disabled}
                      />
                      <Label htmlFor="initial-file-readonly">Read-only in session</Label>
                    </div>

                    <div className="flex flex-wrap items-center gap-2">
                      {selectedCanPreview && (
                        <>
                          <Button
                            type="button"
                            variant={activeView === "source" ? "secondary" : "ghost"}
                            size="sm"
                            onClick={() => setActiveView("source")}
                          >
                            <PencilLine className="mr-2 h-4 w-4" />
                            Source
                          </Button>
                          <Button
                            type="button"
                            variant={activeView === "preview" ? "secondary" : "ghost"}
                            size="sm"
                            onClick={() => setActiveView("preview")}
                          >
                            <Eye className="mr-2 h-4 w-4" />
                            Preview
                          </Button>
                        </>
                      )}
                      <Button
                        type="button"
                        variant="outline"
                        size="sm"
                        onClick={() => replaceInputRef.current?.click()}
                        disabled={disabled}
                      >
                        <Upload className="mr-2 h-4 w-4" />
                        Replace
                      </Button>
                      <Button
                        type="button"
                        variant="outline"
                        size="sm"
                        onClick={() => removeFile(selectedIndex!)}
                        disabled={disabled}
                      >
                        <Trash2 className="mr-2 h-4 w-4" />
                        Remove
                      </Button>
                      <input
                        ref={replaceInputRef}
                        type="file"
                        className="hidden"
                        disabled={disabled}
                        aria-label="Replace selected file with upload"
                        onChange={(event) => {
                          void replaceWithUpload(selectedIndex!, event.target.files);
                          event.target.value = "";
                        }}
                      />
                    </div>
                  </div>
                </div>

                <div className="flex min-h-0 flex-1 flex-col border-t">
                  <div className="border-b px-4 py-2 text-xs text-muted-foreground">
                    <Label htmlFor="initial-file-content" className="text-xs font-normal">
                      {activeView === "preview"
                        ? "Preview"
                        : selectedFile.encoding === "text"
                          ? "Content"
                          : "Base64 Content"}
                    </Label>
                  </div>
                  <div className="min-h-0 flex-1">
                    {activeView === "preview" && selectedCanPreview ? (
                      <InitialFilePreviewPane file={selectedFile} />
                    ) : (
                      <div className="h-full p-4">
                        <Textarea
                          id="initial-file-content"
                          value={selectedFile.content}
                          onChange={(event) =>
                            updateFile(selectedIndex!, { content: event.target.value })
                          }
                          rows={selectedFile.encoding === "text" ? 16 : 12}
                          className="h-full min-h-[280px] resize-none font-mono text-xs"
                          disabled={disabled}
                        />
                      </div>
                    )}
                  </div>
                </div>
              </>
            ) : (
              <div className="flex h-full items-center justify-center p-6 text-sm text-muted-foreground">
                Select a file to edit its contents.
              </div>
            )}
          </div>
        </div>
      )}
    </div>
  );
}

function InitialFilePreviewPane({ file }: { file: InitialFile }) {
  const extension = getFileExtension(file.path);
  const previewType = getInitialFilePreviewType(file);

  if (previewType === "text") {
    return (
      <ScrollArea className="h-full">
        <pre className="min-h-full whitespace-pre-wrap break-words p-4 font-mono text-sm">
          {file.content}
        </pre>
      </ScrollArea>
    );
  }

  if (previewType === "binary") {
    return (
      <div className="flex h-full flex-col items-center justify-center gap-3 p-6 text-center">
        <FileArchive className="h-10 w-10 text-muted-foreground" />
        <div className="space-y-1">
          <p className="text-sm font-medium">Binary file</p>
          <p className="text-sm text-muted-foreground">
            This asset is stored as base64 and does not have an inline preview.
          </p>
        </div>
      </div>
    );
  }

  return (
    <div className="h-full">
      <FilePreview content={file.content} extension={extension} encoding={file.encoding} />
    </div>
  );
}

function renderTreeNode(
  node: TreeNode,
  files: InitialFile[],
  removeFile: (index: number) => void,
  disabled?: boolean,
): ReactNode {
  if (node.kind === "directory") {
    return (
      <FileTreeFolder key={node.id} path={node.id} name={node.name}>
        {node.children.map((child) => renderTreeNode(child, files, removeFile, disabled))}
      </FileTreeFolder>
    );
  }

  const file = files[node.index];
  const previewType = getInitialFilePreviewType(file);

  return (
    <FileTreeFile key={node.id} path={node.id} name={node.name} icon={getPreviewIcon(previewType)}>
      <span className="size-3.5" />
      {getPreviewIcon(previewType)}
      <FileTreeName className="flex-1">{node.name}</FileTreeName>
      <FileTreeActions>
        {file.is_readonly && <Lock className="size-3 text-muted-foreground" />}
        <span className="text-xs text-muted-foreground tabular-nums">
          {formatFileSize(getFileSizeBytes(file))}
        </span>
        <Button
          type="button"
          variant="ghost"
          size="icon"
          className="size-5"
          onClick={() => removeFile(node.index)}
          disabled={disabled}
          title="Remove file"
        >
          <Trash2 className="size-3 text-destructive" />
        </Button>
      </FileTreeActions>
    </FileTreeFile>
  );
}

function buildTree(files: InitialFile[]): { nodes: TreeNode[]; rootDirectoryIds: string[] } {
  const rootNodes: TreeNode[] = [];
  const directories = new Map<string, TreeDirectoryNode>();
  const rootDirectoryIds = new Set<string>();

  const ensureDirectory = (path: string, name: string, parent: TreeDirectoryNode | null) => {
    const id = getDirectoryId(path);
    const existing = directories.get(id);
    if (existing) {
      return existing;
    }

    const directory: TreeDirectoryNode = {
      kind: "directory",
      id,
      name,
      children: [],
    };

    directories.set(id, directory);
    if (parent) {
      parent.children.push(directory);
    } else {
      rootNodes.push(directory);
      rootDirectoryIds.add(id);
    }

    return directory;
  };

  files.forEach((file, index) => {
    const normalizedPath = normalizeInitialFilePath(file.path);
    const segments = normalizedPath.split("/").filter(Boolean);

    if (segments.length === 0) {
      rootNodes.push({
        kind: "file",
        id: getFileId(index),
        index,
        name: getDisplayName(file.path),
      });
      return;
    }

    let parent: TreeDirectoryNode | null = null;
    let currentPath = "";
    const filename = segments[segments.length - 1];

    for (const segment of segments.slice(0, -1)) {
      currentPath = `${currentPath}/${segment}`;
      parent = ensureDirectory(currentPath, segment, parent);
    }

    const fileNode: TreeFileNode = {
      kind: "file",
      id: getFileId(index),
      index,
      name: filename || getDisplayName(file.path),
    };

    if (parent) {
      parent.children.push(fileNode);
    } else {
      rootNodes.push(fileNode);
    }
  });

  const sortNodes = (nodes: TreeNode[]) => {
    nodes.sort((left, right) => {
      if (left.kind !== right.kind) {
        return left.kind === "directory" ? -1 : 1;
      }
      return left.name.localeCompare(right.name);
    });

    nodes.forEach((node) => {
      if (node.kind === "directory") {
        sortNodes(node.children);
      }
    });
  };

  sortNodes(rootNodes);

  return {
    nodes: rootNodes,
    rootDirectoryIds: Array.from(rootDirectoryIds),
  };
}

function getInitialFilePreviewType(file: InitialFile): PreviewType {
  return getPreviewType(getFileExtension(file.path), file.encoding);
}

function getPreviewIcon(type: PreviewType | null) {
  switch (type) {
    case "code":
      return <FileCode className="h-4 w-4 text-sky-600 dark:text-sky-400" />;
    case "csv":
      return <FileSpreadsheet className="h-4 w-4 text-emerald-600 dark:text-emerald-400" />;
    case "json":
      return <FileJson className="h-4 w-4 text-amber-600 dark:text-amber-400" />;
    case "image":
      return <FileImage className="h-4 w-4 text-violet-600 dark:text-violet-400" />;
    case "markdown":
    case "text":
      return <FileText className="h-4 w-4 text-muted-foreground" />;
    case "binary":
      return <FileArchive className="h-4 w-4 text-muted-foreground" />;
    default:
      return <File className="h-4 w-4 text-muted-foreground" />;
  }
}

function getFileId(index: number) {
  return `file:${index}`;
}

function getDirectoryId(path: string) {
  return `dir:${path}`;
}

function getDisplayName(path: string) {
  const normalized = normalizeInitialFilePath(path);
  const name = normalized.split("/").filter(Boolean).pop();
  return name || "untitled";
}

function getFileExtension(path: string) {
  const name = getDisplayName(path);
  const extension = name.includes(".") ? name.split(".").pop() : "";
  return extension?.toLowerCase() ?? "";
}

function normalizeInitialFilePath(path: string) {
  if (!path.trim()) {
    return "/";
  }

  return path.startsWith("/") ? path : `/${path}`;
}

function getFileSizeBytes(file: InitialFile) {
  if (file.encoding === "text") {
    return new Blob([file.content]).size;
  }

  const normalized = file.content.replace(/\s+/g, "");
  const padding = normalized.endsWith("==") ? 2 : normalized.endsWith("=") ? 1 : 0;
  return Math.max(0, Math.floor((normalized.length * 3) / 4) - padding);
}

async function encodeFile(file: File): Promise<Pick<InitialFile, "content" | "encoding">> {
  const bytes = new Uint8Array(await file.arrayBuffer());
  const sample = bytes.slice(0, Math.min(8192, bytes.length));
  const isBinary = sample.includes(0);

  if (!isBinary) {
    return {
      content: await file.text(),
      encoding: "text",
    };
  }

  return {
    content: bytesToBase64(bytes),
    encoding: "base64",
  };
}

function bytesToBase64(bytes: Uint8Array): string {
  let binary = "";
  for (const byte of bytes) {
    binary += String.fromCharCode(byte);
  }
  return btoa(binary);
}
