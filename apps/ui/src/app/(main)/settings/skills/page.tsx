"use client";

import { useState } from "react";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
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
import { useSkills, useCreateSkill, useDeleteSkill, useUploadSkill } from "@/hooks/use-skills";
import { Plus, BookOpen, Trash2, Upload, FileText, Archive } from "lucide-react";
import type { Skill } from "@/lib/api/types";

function SkillCard({
  skill,
  onDelete,
}: {
  skill: Skill;
  onDelete: (id: string) => void;
}) {
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
            <CardTitle className="text-lg">{skill.name}</CardTitle>
            <CardDescription className="text-sm">{skill.description}</CardDescription>
          </div>
        </div>
        <Badge
          variant="outline"
          className={
            skill.status === "active" ? "bg-green-100 text-green-800" : "bg-gray-100 text-gray-800"
          }
        >
          {skill.status}
        </Badge>
      </CardHeader>
      <CardContent>
        <div className="space-y-2 text-sm">
          <div className="flex items-center gap-2">
            <Badge variant="secondary" className="text-xs">
              {skill.source_type}
            </Badge>
            <span className="text-muted-foreground">v{skill.version}</span>
          </div>
          {skill.license && (
            <div className="text-muted-foreground">License: {skill.license}</div>
          )}
          {skill.allowed_tools && (
            <div className="text-muted-foreground">Tools: {skill.allowed_tools}</div>
          )}
        </div>
        <div className="flex items-center justify-end gap-2 mt-4">
          <Button
            variant="ghost"
            size="sm"
            className="text-destructive"
            onClick={() => onDelete(skill.id)}
          >
            <Trash2 className="h-4 w-4" />
          </Button>
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
  const { data: skills = [], isLoading, error } = useSkills();
  const deleteSkillMutation = useDeleteSkill();

  const [addSkillOpen, setAddSkillOpen] = useState(false);
  const [uploadSkillOpen, setUploadSkillOpen] = useState(false);

  const handleDeleteSkill = async (id: string) => {
    if (confirm("Are you sure you want to delete this skill?")) {
      await deleteSkillMutation.mutateAsync(id);
    }
  };

  return (
    <div className="space-y-8">
      <section>
        <div className="flex items-center justify-between mb-4">
          <div>
            <h2 className="text-xl font-semibold">Skills</h2>
            <p className="text-sm text-muted-foreground">
              Manage agent skills. Skills are portable instruction packages following the
              agentskills.io format.
            </p>
          </div>
          <div className="flex gap-2">
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

        {error && (
          <div className="bg-destructive/10 text-destructive p-4 rounded-lg mb-4">
            Failed to load skills: {error.message}
          </div>
        )}

        {isLoading ? (
          <div className="grid gap-4 md:grid-cols-2 lg:grid-cols-3">
            {[...Array(3)].map((_, i) => (
              <SkillCardSkeleton key={i} />
            ))}
          </div>
        ) : skills.length === 0 ? (
          <Card className="p-8 text-center">
            <BookOpen className="h-12 w-12 mx-auto text-muted-foreground mb-4" />
            <h3 className="text-lg font-medium mb-2">No skills configured</h3>
            <p className="text-muted-foreground mb-4">
              Add a skill to provide reusable instructions and tools for your agents.
            </p>
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
          </Card>
        ) : (
          <div className="grid gap-4 md:grid-cols-2 lg:grid-cols-3">
            {skills.map((skill) => (
              <SkillCard key={skill.id} skill={skill} onDelete={handleDeleteSkill} />
            ))}
          </div>
        )}
      </section>

      <AddSkillDialog open={addSkillOpen} onOpenChange={setAddSkillOpen} />
      <UploadSkillDialog open={uploadSkillOpen} onOpenChange={setUploadSkillOpen} />
    </div>
  );
}
