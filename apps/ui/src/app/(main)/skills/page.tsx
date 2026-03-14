"use client";

import { useState } from "react";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Checkbox } from "@/components/ui/checkbox";
import { Skeleton } from "@/components/ui/skeleton";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  useSkills,
  useCreateSkill,
  useDestroySkill,
  useUpdateSkill,
  useUploadSkill,
} from "@/hooks/use-skills";
import { usePolicies } from "@/hooks/use-policies";
import { Plus, BookOpen, Trash2, Upload, FileText, Archive, Box } from "lucide-react";
import type { Skill } from "@/lib/api/types";
import { getEntityNameClassName, getEntityStatusBadgeVariant } from "@/lib/entity-lifecycle";

function SkillCard({
  skill,
  canDestroy,
  onDelete,
}: {
  skill: Skill;
  canDestroy: boolean;
  onDelete: (skill: Skill) => void;
}) {
  const updateSkill = useUpdateSkill(skill.id);
  const isArchived = skill.status === "archived";
  const isDeleted = skill.status === "deleted";

  return (
    <Card>
      <CardHeader className="flex flex-row items-start justify-between space-y-0">
        <div className="flex items-center gap-3">
          <div className="flex h-9 w-9 items-center justify-center rounded-lg bg-primary/10">
            {skill.source_type === "archive" ? (
              <Archive className="h-5 w-5 text-primary" />
            ) : (
              <FileText className="h-5 w-5 text-primary" />
            )}
          </div>
          <div>
            <CardTitle className={`text-lg ${getEntityNameClassName(skill.status)}`}>
              {skill.name}
            </CardTitle>
            <CardDescription className="text-sm">{skill.description}</CardDescription>
          </div>
        </div>
        <Badge variant={getEntityStatusBadgeVariant(skill.status)}>{skill.status}</Badge>
      </CardHeader>
      <CardContent>
        <div className="space-y-2 text-sm">
          <div className="flex items-center gap-2">
            <Badge variant="secondary" className="text-xs">
              {skill.source_type}
            </Badge>
            <span className="text-muted-foreground">v{skill.version}</span>
          </div>
          {skill.license && <div className="text-muted-foreground">License: {skill.license}</div>}
          {skill.allowed_tools && (
            <div className="text-muted-foreground">Tools: {skill.allowed_tools}</div>
          )}
        </div>
        <div className="flex items-center justify-end gap-2 mt-4">
          {!isArchived && !isDeleted && (
            <Button
              variant="outline"
              size="sm"
              onClick={() => updateSkill.mutate({ status: "archived" })}
              disabled={updateSkill.isPending}
            >
              <Archive className="h-4 w-4 mr-1" />
              {updateSkill.isPending ? "Archiving..." : "Archive"}
            </Button>
          )}
          {isArchived && canDestroy && (
            <Button variant="destructive" size="sm" onClick={() => onDelete(skill)}>
              <Trash2 className="h-4 w-4 mr-1" />
              Delete
            </Button>
          )}
        </div>
      </CardContent>
    </Card>
  );
}

function AddSkillDialog({
  open,
  onOpenChange,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const [skillMd, setSkillMd] = useState(
    `---\nname: my-skill\ndescription: Describe what this skill does.\n---\n\n# My Skill\n\nInstructions here.\n`,
  );

  const createSkill = useCreateSkill();

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    await createSkill.mutateAsync({ skill_md: skillMd });
    onOpenChange(false);
    setSkillMd(
      `---\nname: my-skill\ndescription: Describe what this skill does.\n---\n\n# My Skill\n\nInstructions here.\n`,
    );
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-2xl">
        <DialogHeader>
          <DialogTitle>Add Skill</DialogTitle>
          <DialogDescription>
            Create a new skill from SKILL.md content. Use YAML frontmatter for metadata and markdown
            for instructions.
          </DialogDescription>
        </DialogHeader>
        <form onSubmit={handleSubmit} className="space-y-4">
          <div className="space-y-2">
            <Label htmlFor="skill-md">SKILL.md</Label>
            <Textarea
              id="skill-md"
              value={skillMd}
              onChange={(e: React.ChangeEvent<HTMLTextAreaElement>) => setSkillMd(e.target.value)}
              placeholder="---&#10;name: my-skill&#10;description: ...&#10;---&#10;&#10;Instructions..."
              rows={14}
              className="font-mono text-sm"
              required
            />
          </div>
          <DialogFooter>
            <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
              Cancel
            </Button>
            <Button type="submit" disabled={createSkill.isPending || !skillMd.trim()}>
              {createSkill.isPending ? "Creating..." : "Create Skill"}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

function UploadSkillDialog({
  open,
  onOpenChange,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const [file, setFile] = useState<File | null>(null);
  const uploadSkill = useUploadSkill();

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!file) return;
    await uploadSkill.mutateAsync(file);
    onOpenChange(false);
    setFile(null);
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Upload Skill Archive</DialogTitle>
          <DialogDescription>
            Upload a ZIP archive containing a SKILL.md file and optional scripts, references, and
            assets.
          </DialogDescription>
        </DialogHeader>
        <form onSubmit={handleSubmit} className="space-y-4">
          <div className="space-y-2">
            <Label htmlFor="skill-file">ZIP Archive</Label>
            <input
              id="skill-file"
              type="file"
              accept=".zip"
              onChange={(e) => setFile(e.target.files?.[0] ?? null)}
              className="block w-full text-sm text-muted-foreground file:mr-4 file:py-2 file:px-4 file:rounded-md file:border-0 file:text-sm file:font-medium file:bg-primary file:text-primary-foreground hover:file:bg-primary/90"
            />
            <p className="text-xs text-muted-foreground">Maximum archive size: 10 MB</p>
          </div>
          <DialogFooter>
            <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
              Cancel
            </Button>
            <Button type="submit" disabled={uploadSkill.isPending || !file}>
              {uploadSkill.isPending ? "Uploading..." : "Upload"}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

function SkillCardSkeleton() {
  return (
    <Card>
      <CardHeader className="flex flex-row items-start justify-between space-y-0">
        <div className="flex items-center gap-3">
          <Skeleton className="h-9 w-9 rounded-lg" />
          <div className="space-y-2">
            <Skeleton className="h-5 w-32" />
            <Skeleton className="h-4 w-48" />
          </div>
        </div>
        <Skeleton className="h-5 w-16" />
      </CardHeader>
      <CardContent>
        <Skeleton className="h-4 w-full mb-4" />
        <Skeleton className="h-8 w-24 ml-auto" />
      </CardContent>
    </Card>
  );
}

export default function SkillsPage() {
  const [showArchived, setShowArchived] = useState(false);
  const { data: skills = [], isLoading, error } = useSkills({ includeArchived: showArchived });
  const destroySkillMutation = useDestroySkill();
  const { can: canPolicies } = usePolicies("skills");
  const canDestroy = canPolicies("skill.dangerous");

  const [addSkillOpen, setAddSkillOpen] = useState(false);
  const [uploadSkillOpen, setUploadSkillOpen] = useState(false);
  const [pendingDeleteSkill, setPendingDeleteSkill] = useState<Skill | null>(null);

  const handleDeleteSkill = async () => {
    if (!pendingDeleteSkill) return;
    await destroySkillMutation.mutateAsync(pendingDeleteSkill.id);
    setPendingDeleteSkill(null);
  };

  return (
    <div className="container mx-auto p-6">
      <div className="flex items-center justify-between mb-6">
        <div>
          <h1 className="text-2xl font-bold">Skills</h1>
          <label className="mt-2 inline-flex items-center gap-2 text-sm text-muted-foreground">
            <Checkbox checked={showArchived} onCheckedChange={setShowArchived} />
            Show archived skills
          </label>
        </div>
        <div className="flex gap-2">
          <Button variant="outline" onClick={() => setUploadSkillOpen(true)}>
            <Upload className="h-4 w-4 mr-2" />
            Upload ZIP
          </Button>
          <Button variant="accent" onClick={() => setAddSkillOpen(true)}>
            <Plus className="h-4 w-4 mr-2" />
            Add Skill
          </Button>
        </div>
      </div>

      {error && (
        <div className="mb-4 p-4 bg-red-50 border border-red-200 rounded-md text-red-600 text-sm">
          Failed to load skills: {error.message}
        </div>
      )}

      {isLoading ? (
        <div className="grid gap-4 md:grid-cols-2 lg:grid-cols-3">
          {[...Array(6)].map((_, i) => (
            <SkillCardSkeleton key={i} />
          ))}
        </div>
      ) : skills.length === 0 ? (
        <div className="text-center py-12">
          <BookOpen className="h-12 w-12 mx-auto text-muted-foreground mb-4" />
          <p className="text-muted-foreground mb-4">No skills yet</p>
          <div className="flex justify-center gap-2">
            <Button variant="outline" onClick={() => setUploadSkillOpen(true)}>
              <Upload className="h-4 w-4 mr-2" />
              Upload ZIP
            </Button>
            <Button onClick={() => setAddSkillOpen(true)}>
              <Plus className="h-4 w-4 mr-2" />
              Add Skill
            </Button>
          </div>
        </div>
      ) : (
        <div className="grid gap-4 md:grid-cols-2 lg:grid-cols-3">
          {skills.map((skill) => (
            <SkillCard
              key={skill.id}
              skill={skill}
              canDestroy={canDestroy}
              onDelete={setPendingDeleteSkill}
            />
          ))}
        </div>
      )}

      <AddSkillDialog open={addSkillOpen} onOpenChange={setAddSkillOpen} />
      <UploadSkillDialog open={uploadSkillOpen} onOpenChange={setUploadSkillOpen} />
      <Dialog
        open={pendingDeleteSkill !== null}
        onOpenChange={(open) => !open && setPendingDeleteSkill(null)}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Delete Skill</DialogTitle>
            <DialogDescription>
              Permanently delete the archived skill{" "}
              <span className="font-medium">{pendingDeleteSkill?.name}</span>? Existing references
              will render as deleted tombstones.
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" onClick={() => setPendingDeleteSkill(null)}>
              Cancel
            </Button>
            <Button
              variant="destructive"
              onClick={handleDeleteSkill}
              disabled={destroySkillMutation.isPending}
            >
              <Box className="h-4 w-4 mr-1" />
              {destroySkillMutation.isPending ? "Deleting..." : "Delete Skill"}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
