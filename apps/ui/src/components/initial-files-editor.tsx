"use client";

import { useMemo, useRef } from "react";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import { Badge } from "@/components/ui/badge";
import { Upload, Plus, Trash2, FileText, FileArchive } from "lucide-react";
import type { InitialFile } from "@/lib/api/types";

interface InitialFilesEditorProps {
  value: InitialFile[];
  onChange: (files: InitialFile[]) => void;
  disabled?: boolean;
  title?: string;
  description?: string;
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
  const fileInputRef = useRef<HTMLInputElement>(null);

  const files = useMemo(() => value ?? [], [value]);

  const updateFile = (index: number, patch: Partial<InitialFile>) => {
    const next = [...files];
    next[index] = { ...next[index], ...patch };
    onChange(next);
  };

  const removeFile = (index: number) => {
    onChange(files.filter((_, current) => current !== index));
  };

  const addTextFile = () => {
    onChange([...files, { ...EMPTY_TEXT_FILE, path: `/file-${files.length + 1}.txt` }]);
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

    onChange([...files, ...uploaded]);
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
  };

  return (
    <div className="space-y-4">
      <div className="space-y-1">
        <Label>{title}</Label>
        <p className="text-xs text-muted-foreground">{description}</p>
      </div>

      <div className="flex flex-wrap gap-2">
        <Button type="button" variant="outline" size="sm" onClick={addTextFile} disabled={disabled}>
          <Plus className="w-4 h-4 mr-2" />
          Add Text File
        </Button>
        <Button
          type="button"
          variant="outline"
          size="sm"
          onClick={() => fileInputRef.current?.click()}
          disabled={disabled}
        >
          <Upload className="w-4 h-4 mr-2" />
          Upload File
        </Button>
        <input
          ref={fileInputRef}
          type="file"
          multiple
          className="hidden"
          onChange={(event) => {
            void addUploadedFiles(event.target.files);
            event.target.value = "";
          }}
        />
      </div>

      {files.length === 0 ? (
        <div className="rounded-lg border border-dashed p-4 text-sm text-muted-foreground">
          No starter files configured.
        </div>
      ) : (
        <div className="space-y-3">
          {files.map((file, index) => (
            <div key={`${file.path}-${index}`} className="rounded-lg border p-4 space-y-3">
              <div className="flex items-center justify-between gap-3">
                <div className="flex items-center gap-2 min-w-0">
                  {file.encoding === "text" ? (
                    <FileText className="w-4 h-4 text-muted-foreground" />
                  ) : (
                    <FileArchive className="w-4 h-4 text-muted-foreground" />
                  )}
                  <span className="font-medium truncate">{file.path}</span>
                  <Badge variant="secondary">{file.encoding}</Badge>
                  {file.is_readonly && <Badge variant="outline">read-only</Badge>}
                </div>
                <Button
                  type="button"
                  variant="ghost"
                  size="icon"
                  onClick={() => removeFile(index)}
                  disabled={disabled}
                >
                  <Trash2 className="w-4 h-4" />
                </Button>
              </div>

              <div className="space-y-2">
                <Label htmlFor={`initial-file-path-${index}`}>Path</Label>
                <Input
                  id={`initial-file-path-${index}`}
                  value={file.path}
                  onChange={(event) => updateFile(index, { path: event.target.value })}
                  placeholder="/.agents/skills/my-skill/SKILL.md"
                  disabled={disabled}
                />
                <p className="text-xs text-muted-foreground">
                  Absolute path inside the session workspace. `/workspace/...` is also accepted.
                </p>
              </div>

              <div className="flex items-center gap-2">
                <Checkbox
                  id={`initial-file-readonly-${index}`}
                  checked={file.is_readonly}
                  onCheckedChange={(checked) =>
                    updateFile(index, { is_readonly: checked === true })
                  }
                  disabled={disabled}
                />
                <Label htmlFor={`initial-file-readonly-${index}`}>Read-only in session</Label>
              </div>

              <div className="space-y-2">
                <div className="flex items-center justify-between gap-3">
                  <Label htmlFor={`initial-file-content-${index}`}>
                    {file.encoding === "text" ? "Content" : "Base64 Content"}
                  </Label>
                  <label className="text-xs text-muted-foreground cursor-pointer">
                    <input
                      type="file"
                      className="hidden"
                      disabled={disabled}
                      onChange={(event) => {
                        void replaceWithUpload(index, event.target.files);
                        event.target.value = "";
                      }}
                    />
                    Replace from file
                  </label>
                </div>
                <Textarea
                  id={`initial-file-content-${index}`}
                  value={file.content}
                  onChange={(event) => updateFile(index, { content: event.target.value })}
                  rows={file.encoding === "text" ? 8 : 5}
                  className="font-mono text-xs"
                  disabled={disabled}
                />
              </div>
            </div>
          ))}
        </div>
      )}
    </div>
  );
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
