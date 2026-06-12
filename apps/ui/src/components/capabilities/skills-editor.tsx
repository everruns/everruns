"use client";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import { Switch } from "@/components/ui/switch";
import { Separator } from "@/components/ui/separator";
import { Plus, Sparkles, Trash2, X } from "lucide-react";
import type { SkillEntry } from "./declarative-form-types";
import { newSkillEntry } from "./declarative-form-types";

const MAX_SKILLS = 16;

let fileCounter = 0;
function fileId(): string {
  fileCounter += 1;
  return `skill-file-${fileCounter}-${Date.now()}`;
}

export function SkillsEditor({
  skills,
  onChange,
}: {
  skills: SkillEntry[];
  onChange: (skills: SkillEntry[]) => void;
}) {
  const update = (id: string, patch: Partial<SkillEntry>) =>
    onChange(skills.map((s) => (s.id === id ? { ...s, ...patch } : s)));

  return (
    <div className="space-y-4">
      {skills.length === 0 && (
        <p className="text-sm text-muted-foreground">
          No skills. Skills are mounted under <code>/.agents/skills/</code> and can be invoked as
          slash commands.
        </p>
      )}
      {skills.map((skill) => (
        <div key={skill.id} className="space-y-4 border p-4">
          <div className="flex items-start justify-between gap-2">
            <div className="flex items-center gap-2">
              <div className="flex h-8 w-8 items-center justify-center bg-primary/10">
                <Sparkles className="h-4 w-4 text-primary" />
              </div>
              <div className="grid gap-1.5">
                <Label htmlFor={`skill-name-${skill.id}`}>Skill name</Label>
                <Input
                  id={`skill-name-${skill.id}`}
                  placeholder="research-helper"
                  value={skill.name}
                  onChange={(e) => update(skill.id, { name: e.target.value })}
                  className="w-64"
                />
              </div>
            </div>
            <Button
              type="button"
              variant="ghost"
              size="sm"
              className="h-9 w-9 p-0 shrink-0"
              onClick={() => onChange(skills.filter((s) => s.id !== skill.id))}
              aria-label="Remove skill"
            >
              <Trash2 className="h-4 w-4" />
            </Button>
          </div>

          <div className="grid gap-1.5">
            <Label htmlFor={`skill-desc-${skill.id}`}>Description</Label>
            <Input
              id={`skill-desc-${skill.id}`}
              placeholder="When to use this skill"
              value={skill.description}
              onChange={(e) => update(skill.id, { description: e.target.value })}
            />
          </div>

          <div className="grid gap-1.5">
            <Label htmlFor={`skill-instructions-${skill.id}`}>Instructions (SKILL.md body)</Label>
            <Textarea
              id={`skill-instructions-${skill.id}`}
              placeholder="Markdown instructions the model follows when this skill is active."
              value={skill.instructions}
              onChange={(e) => update(skill.id, { instructions: e.target.value })}
              rows={6}
            />
          </div>

          <div className="space-y-2">
            <div className="flex items-center justify-between">
              <Label>Bundled files</Label>
              <Button
                type="button"
                variant="ghost"
                size="sm"
                className="h-7 text-xs"
                onClick={() =>
                  update(skill.id, {
                    files: [...skill.files, { id: fileId(), path: "", content: "" }],
                  })
                }
              >
                <Plus className="h-3 w-3 mr-1" />
                Add file
              </Button>
            </div>
            {skill.files.map((file) => (
              <div key={file.id} className="space-y-2 border p-2">
                <div className="flex items-center gap-2">
                  <Input
                    placeholder="reference.md"
                    value={file.path}
                    onChange={(e) =>
                      update(skill.id, {
                        files: skill.files.map((f) =>
                          f.id === file.id ? { ...f, path: e.target.value } : f,
                        ),
                      })
                    }
                    className="font-mono text-xs"
                  />
                  <Button
                    type="button"
                    variant="ghost"
                    size="sm"
                    className="h-9 w-9 p-0 shrink-0"
                    onClick={() =>
                      update(skill.id, { files: skill.files.filter((f) => f.id !== file.id) })
                    }
                    aria-label="Remove file"
                  >
                    <X className="h-4 w-4" />
                  </Button>
                </div>
                <Textarea
                  placeholder="File content"
                  value={file.content}
                  onChange={(e) =>
                    update(skill.id, {
                      files: skill.files.map((f) =>
                        f.id === file.id ? { ...f, content: e.target.value } : f,
                      ),
                    })
                  }
                  rows={3}
                  className="font-mono text-xs"
                />
              </div>
            ))}
          </div>

          <Separator />

          <div className="flex items-center justify-between">
            <div className="space-y-0.5">
              <Label htmlFor={`skill-invocable-${skill.id}`}>User-invocable</Label>
              <p className="text-xs text-muted-foreground">
                Appears as a <code>/{skill.name || "name"}</code> slash command.
              </p>
            </div>
            <Switch
              id={`skill-invocable-${skill.id}`}
              checked={skill.userInvocable}
              onCheckedChange={(checked) => update(skill.id, { userInvocable: checked })}
            />
          </div>
          <div className="flex items-center justify-between">
            <div className="space-y-0.5">
              <Label htmlFor={`skill-no-model-${skill.id}`}>Disable model invocation</Label>
              <p className="text-xs text-muted-foreground">
                Prevent the model from auto-activating this skill.
              </p>
            </div>
            <Switch
              id={`skill-no-model-${skill.id}`}
              checked={skill.disableModelInvocation}
              onCheckedChange={(checked) => update(skill.id, { disableModelInvocation: checked })}
            />
          </div>
        </div>
      ))}

      <Button
        type="button"
        variant="outline"
        size="sm"
        disabled={skills.length >= MAX_SKILLS}
        onClick={() => onChange([...skills, newSkillEntry()])}
      >
        <Plus className="h-4 w-4 mr-1" />
        Add skill
      </Button>
    </div>
  );
}
