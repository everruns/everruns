"use client";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { FileText, Plus, Trash2 } from "lucide-react";
import type { FileEntry } from "./declarative-form-types";
import { newFileEntry } from "./declarative-form-types";

const MAX_FILES = 32;

export function FilesEditor({
  files,
  onChange,
}: {
  files: FileEntry[];
  onChange: (files: FileEntry[]) => void;
}) {
  const update = (id: string, patch: Partial<FileEntry>) =>
    onChange(files.map((f) => (f.id === id ? { ...f, ...patch } : f)));

  return (
    <div className="space-y-4">
      {files.length === 0 && (
        <p className="text-sm text-muted-foreground">
          No files. Text files are mounted into the session filesystem at the given absolute path.
        </p>
      )}
      {files.map((file) => (
        <div key={file.id} className="space-y-3 border p-4">
          <div className="flex items-start justify-between gap-2">
            <div className="flex flex-1 items-center gap-2">
              <div className="flex h-8 w-8 items-center justify-center bg-primary/10 shrink-0">
                <FileText className="h-4 w-4 text-primary" />
              </div>
              <div className="grid flex-1 gap-1.5">
                <Label htmlFor={`file-path-${file.id}`}>Mount path</Label>
                <Input
                  id={`file-path-${file.id}`}
                  placeholder="/docs/README.md"
                  value={file.path}
                  onChange={(e) => update(file.id, { path: e.target.value })}
                  className="font-mono text-xs"
                />
              </div>
            </div>
            <div className="grid gap-1.5">
              <Label>Access</Label>
              <Select
                value={file.access}
                onValueChange={(value) => update(file.id, { access: value as FileEntry["access"] })}
              >
                <SelectTrigger className="w-36">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="readonly">Read-only</SelectItem>
                  <SelectItem value="readwrite">Read-write</SelectItem>
                </SelectContent>
              </Select>
            </div>
            <Button
              type="button"
              variant="ghost"
              size="sm"
              className="h-9 w-9 p-0 shrink-0 self-end"
              onClick={() => onChange(files.filter((f) => f.id !== file.id))}
              aria-label="Remove file"
            >
              <Trash2 className="h-4 w-4" />
            </Button>
          </div>
          <Textarea
            placeholder="File content"
            value={file.content}
            onChange={(e) => update(file.id, { content: e.target.value })}
            rows={5}
            className="font-mono text-xs"
          />
        </div>
      ))}

      <Button
        type="button"
        variant="outline"
        size="sm"
        disabled={files.length >= MAX_FILES}
        onClick={() => onChange([...files, newFileEntry()])}
      >
        <Plus className="h-4 w-4 mr-1" />
        Add file
      </Button>
    </div>
  );
}
