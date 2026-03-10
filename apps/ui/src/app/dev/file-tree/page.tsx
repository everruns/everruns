"use client";

import { useState } from "react";
import {
  FileTree,
  FileTreeFolder,
  FileTreeFile,
  FileTreeActions,
  FileTreeName,
} from "@/components/ai-elements/file-tree";
import { Button } from "@/components/ui/button";
import { FileCode, FileJson, FileText, Image, File, Trash2, Plus, FolderPlus } from "lucide-react";

function getFileIcon(filename: string) {
  const ext = filename.split(".").pop()?.toLowerCase();
  switch (ext) {
    case "ts":
    case "tsx":
    case "js":
    case "jsx":
      return <FileCode className="size-4 text-sky-600 dark:text-sky-400" />;
    case "json":
      return <FileJson className="size-4 text-amber-600 dark:text-amber-400" />;
    case "md":
      return <FileText className="size-4 text-muted-foreground" />;
    case "png":
    case "jpg":
      return <Image className="size-4 text-violet-600 dark:text-violet-400" />;
    default:
      return <File className="size-4 text-muted-foreground" />;
  }
}

export default function FileTreeDevPage() {
  const [selectedPath, setSelectedPath] = useState<string | undefined>();
  const [expanded, setExpanded] = useState<Set<string>>(new Set(["/workspace", "/workspace/src"]));

  return (
    <div className="p-8 max-w-5xl mx-auto space-y-8">
      <div>
        <h1 className="text-2xl font-bold mb-2">FileTree Component</h1>
        <p className="text-muted-foreground">AI Elements FileTree styled for Slate Design System</p>
      </div>

      <div className="grid grid-cols-2 gap-8">
        {/* Basic FileTree */}
        <div className="space-y-4">
          <h2 className="text-lg font-semibold">Basic FileTree</h2>
          <div className="border bg-card">
            <FileTree
              selectedPath={selectedPath}
              onSelect={setSelectedPath}
              expanded={expanded}
              onExpandedChange={setExpanded}
            >
              <FileTreeFolder path="/workspace" name="workspace">
                <FileTreeFolder path="/workspace/src" name="src">
                  <FileTreeFile
                    path="/workspace/src/index.ts"
                    name="index.ts"
                    icon={getFileIcon("index.ts")}
                  />
                  <FileTreeFile
                    path="/workspace/src/app.tsx"
                    name="app.tsx"
                    icon={getFileIcon("app.tsx")}
                  />
                  <FileTreeFolder path="/workspace/src/components" name="components">
                    <FileTreeFile
                      path="/workspace/src/components/button.tsx"
                      name="button.tsx"
                      icon={getFileIcon("button.tsx")}
                    />
                    <FileTreeFile
                      path="/workspace/src/components/card.tsx"
                      name="card.tsx"
                      icon={getFileIcon("card.tsx")}
                    />
                  </FileTreeFolder>
                </FileTreeFolder>
                <FileTreeFile
                  path="/workspace/package.json"
                  name="package.json"
                  icon={getFileIcon("package.json")}
                />
                <FileTreeFile
                  path="/workspace/README.md"
                  name="README.md"
                  icon={getFileIcon("README.md")}
                />
              </FileTreeFolder>
            </FileTree>
          </div>
          <div className="text-sm text-muted-foreground font-mono">
            Selected: {selectedPath || "none"}
          </div>
        </div>

        {/* FileTree with Actions */}
        <div className="space-y-4">
          <h2 className="text-lg font-semibold">With Actions</h2>
          <div className="border bg-card">
            <FileTree defaultExpanded={new Set(["/project", "/project/lib"])}>
              <FileTreeFolder path="/project" name="project">
                <FileTreeFolder path="/project/lib" name="lib">
                  <FileTreeFile
                    path="/project/lib/utils.ts"
                    name="utils.ts"
                    icon={getFileIcon("utils.ts")}
                  >
                    <span className="size-3.5" />
                    {getFileIcon("utils.ts")}
                    <FileTreeName className="flex-1">utils.ts</FileTreeName>
                    <FileTreeActions>
                      <span className="text-xs text-muted-foreground tabular-nums">2.1 KB</span>
                      <Button variant="ghost" size="icon" className="size-5">
                        <Trash2 className="size-3 text-destructive" />
                      </Button>
                    </FileTreeActions>
                  </FileTreeFile>
                  <FileTreeFile
                    path="/project/lib/api.ts"
                    name="api.ts"
                    icon={getFileIcon("api.ts")}
                  >
                    <span className="size-3.5" />
                    {getFileIcon("api.ts")}
                    <FileTreeName className="flex-1">api.ts</FileTreeName>
                    <FileTreeActions>
                      <span className="text-xs text-muted-foreground tabular-nums">4.5 KB</span>
                      <Button variant="ghost" size="icon" className="size-5">
                        <Trash2 className="size-3 text-destructive" />
                      </Button>
                    </FileTreeActions>
                  </FileTreeFile>
                </FileTreeFolder>
                <FileTreeFolder
                  path="/project/assets"
                  name="assets"
                  actions={
                    <FileTreeActions>
                      <Button variant="ghost" size="icon" className="size-5">
                        <Plus className="size-3" />
                      </Button>
                      <Button variant="ghost" size="icon" className="size-5">
                        <FolderPlus className="size-3" />
                      </Button>
                    </FileTreeActions>
                  }
                >
                  <FileTreeFile
                    path="/project/assets/logo.png"
                    name="logo.png"
                    icon={getFileIcon("logo.png")}
                  />
                </FileTreeFolder>
              </FileTreeFolder>
            </FileTree>
          </div>
        </div>
      </div>

      {/* Design Notes */}
      <div className="space-y-4">
        <h2 className="text-lg font-semibold">Slate Design System</h2>
        <div className="grid grid-cols-3 gap-4 text-sm">
          <div className="border p-4 bg-card">
            <h3 className="font-medium mb-2">Sharp Corners</h3>
            <p className="text-muted-foreground">0px radius on all elements</p>
          </div>
          <div className="border p-4 bg-card">
            <h3 className="font-medium mb-2">Grayscale + Accent</h3>
            <p className="text-muted-foreground">
              Folders: <span className="text-amber-500">amber</span>, Files by type
            </p>
          </div>
          <div className="border p-4 bg-card">
            <h3 className="font-medium mb-2">Compact Spacing</h3>
            <p className="text-muted-foreground">py-1, gap-1.5 for dense trees</p>
          </div>
        </div>
      </div>

      {/* Usage Code */}
      <div className="space-y-4">
        <h2 className="text-lg font-semibold">Usage</h2>
        <pre className="p-4 bg-muted text-sm overflow-x-auto font-mono">
          {`import {
  FileTree,
  FileTreeFolder,
  FileTreeFile,
  FileTreeActions,
} from "@/components/ai-elements/file-tree";

<FileTree
  selectedPath={selectedPath}
  onSelect={setSelectedPath}
  expanded={expanded}
  onExpandedChange={setExpanded}
>
  <FileTreeFolder path="/workspace" name="workspace">
    <FileTreeFile path="/workspace/file.ts" name="file.ts" />
  </FileTreeFolder>
</FileTree>`}
        </pre>
      </div>
    </div>
  );
}
