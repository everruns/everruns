"use client";

import { useState } from "react";
import {
  FileTree,
  FileTreeFolder,
  FileTreeFile,
  FileTreeActions,
} from "@/components/ai-elements/file-tree";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import {
  FileCode,
  FileJson,
  FileText,
  Image,
  File,
  Trash2,
  Plus,
  FolderPlus,
} from "lucide-react";

function getFileIcon(filename: string) {
  const ext = filename.split(".").pop()?.toLowerCase();
  switch (ext) {
    case "ts":
    case "tsx":
    case "js":
    case "jsx":
      return <FileCode className="size-4 text-blue-500" />;
    case "json":
      return <FileJson className="size-4 text-yellow-500" />;
    case "md":
      return <FileText className="size-4 text-gray-500" />;
    case "png":
    case "jpg":
      return <Image className="size-4 text-purple-500" />;
    default:
      return <File className="size-4 text-muted-foreground" />;
  }
}

export default function FileTreeDevPage() {
  const [selectedPath, setSelectedPath] = useState<string | undefined>();
  const [expanded, setExpanded] = useState<Set<string>>(
    new Set(["/workspace", "/workspace/src"])
  );

  return (
    <div className="p-8 max-w-4xl mx-auto space-y-8">
      <div>
        <h1 className="text-2xl font-bold mb-2">FileTree Component</h1>
        <p className="text-muted-foreground">
          AI Elements FileTree component for workspace file browsing
        </p>
      </div>

      <div className="grid grid-cols-2 gap-8">
        {/* Basic FileTree */}
        <div className="space-y-4">
          <h2 className="text-lg font-semibold">Basic FileTree</h2>
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
          <div className="text-sm text-muted-foreground">
            Selected: {selectedPath || "none"}
          </div>
        </div>

        {/* FileTree with Actions */}
        <div className="space-y-4">
          <h2 className="text-lg font-semibold">With Actions & Badges</h2>
          <FileTree
            defaultExpanded={new Set(["/project", "/project/lib"])}
          >
            <FileTreeFolder path="/project" name="project">
              <FileTreeFolder path="/project/lib" name="lib">
                <FileTreeFile
                  path="/project/lib/utils.ts"
                  name="utils.ts"
                  icon={getFileIcon("utils.ts")}
                >
                  <span className="size-4" />
                  {getFileIcon("utils.ts")}
                  <span className="flex-1 truncate">utils.ts</span>
                  <FileTreeActions>
                    <Badge variant="outline" className="text-xs py-0 h-4">
                      2.1 KB
                    </Badge>
                    <Button variant="ghost" size="icon" className="h-5 w-5">
                      <Trash2 className="h-3 w-3 text-destructive" />
                    </Button>
                  </FileTreeActions>
                </FileTreeFile>
                <FileTreeFile
                  path="/project/lib/api.ts"
                  name="api.ts"
                  icon={getFileIcon("api.ts")}
                >
                  <span className="size-4" />
                  {getFileIcon("api.ts")}
                  <span className="flex-1 truncate">api.ts</span>
                  <FileTreeActions>
                    <Badge variant="outline" className="text-xs py-0 h-4">
                      4.5 KB
                    </Badge>
                    <Button variant="ghost" size="icon" className="h-5 w-5">
                      <Trash2 className="h-3 w-3 text-destructive" />
                    </Button>
                  </FileTreeActions>
                </FileTreeFile>
              </FileTreeFolder>
              <FileTreeFolder path="/project/assets" name="assets">
                <FileTreeActions>
                  <Button variant="ghost" size="icon" className="h-5 w-5">
                    <Plus className="h-3 w-3" />
                  </Button>
                  <Button variant="ghost" size="icon" className="h-5 w-5">
                    <FolderPlus className="h-3 w-3" />
                  </Button>
                </FileTreeActions>
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

      {/* Usage Code */}
      <div className="space-y-4">
        <h2 className="text-lg font-semibold">Usage</h2>
        <pre className="p-4 bg-muted rounded-lg text-sm overflow-x-auto">
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
