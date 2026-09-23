/**
 * Project switcher — the prime top slot of the sidebar (Direction F).
 *
 * Projects are switched constantly, so they take the position the org chip used
 * to occupy. A "Projects" label + "＋" sit above a prominent switcher. The org
 * moved into the bottom user menu (see sidebar-user-menu).
 */
"use client";

import { useState } from "react";
import { Check, ChevronsUpDown, FolderGit2, Plus } from "lucide-react";
import { useCreateProject } from "@/hooks/use-projects";
import { useProject } from "@/providers/project-provider";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuPositioner,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";

function CreateProjectDialog({
  open,
  onOpenChange,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const [name, setName] = useState("");
  const createProject = useCreateProject();
  const { setCurrentProject } = useProject();

  const handleSubmit = async (event: React.FormEvent) => {
    event.preventDefault();
    if (!name.trim()) return;
    const project = await createProject.mutateAsync({ name: name.trim() });
    setCurrentProject(project);
    setName("");
    onOpenChange(false);
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Create project</DialogTitle>
          <DialogDescription>
            Projects group agents, skills, apps, sessions and more within this organization.
          </DialogDescription>
        </DialogHeader>
        <form onSubmit={handleSubmit} className="space-y-4">
          <div className="space-y-2">
            <Label htmlFor="project-name">Name</Label>
            <Input
              id="project-name"
              value={name}
              onChange={(event) => setName(event.target.value)}
              placeholder="e.g. Support"
              required
            />
          </div>
          {createProject.isError && (
            <p className="text-sm text-destructive">
              Failed to create project: {createProject.error.message}
            </p>
          )}
          <DialogFooter>
            <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
              Cancel
            </Button>
            <Button type="submit" disabled={createProject.isPending || !name.trim()}>
              {createProject.isPending ? "Creating..." : "Create"}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

export function SidebarProjectMenu() {
  const { currentProject, projects, setCurrentProject } = useProject();
  const [createOpen, setCreateOpen] = useState(false);

  return (
    <>
      <div className="border-b border-border/70 px-2.5 py-2">
        <div className="flex items-center justify-between px-1 pb-1">
          <span className="font-mono text-[10px] uppercase tracking-[0.18em] text-muted-foreground">
            Projects
          </span>
          <button
            type="button"
            aria-label="Create project"
            onClick={() => setCreateOpen(true)}
            className="flex h-5 w-5 items-center justify-center border border-input text-muted-foreground transition-colors hover:border-border hover:text-foreground"
          >
            <Plus className="h-3.5 w-3.5" />
          </button>
        </div>
        <DropdownMenu>
          <DropdownMenuTrigger className="flex w-full items-center gap-2 border border-input px-2.5 py-2 text-[13px] font-semibold transition-colors hover:border-border hover:bg-card">
            <FolderGit2 className="icon-sharp h-4 w-4 text-muted-foreground" />
            <span className="flex-1 truncate text-left leading-5">
              {currentProject?.name ?? "Select project"}
            </span>
            <ChevronsUpDown className="icon-sharp h-3.5 w-3.5 text-muted-foreground" />
          </DropdownMenuTrigger>
          <DropdownMenuPositioner side="bottom" align="start">
            <DropdownMenuContent className="w-56">
              <DropdownMenuGroup>
                <DropdownMenuLabel>Projects</DropdownMenuLabel>
                {projects.map((project) => (
                  <DropdownMenuItem
                    key={project.id}
                    onClick={() => setCurrentProject(project)}
                    className="flex items-center justify-between"
                  >
                    <span className="truncate">{project.name}</span>
                    {currentProject?.id === project.id && (
                      <Check className="icon-sharp h-4 w-4 text-primary" />
                    )}
                  </DropdownMenuItem>
                ))}
              </DropdownMenuGroup>
              <DropdownMenuSeparator />
              <DropdownMenuItem onClick={() => setCreateOpen(true)}>
                <Plus className="icon-sharp mr-2 h-4 w-4" />
                New project
              </DropdownMenuItem>
            </DropdownMenuContent>
          </DropdownMenuPositioner>
        </DropdownMenu>
      </div>

      <CreateProjectDialog open={createOpen} onOpenChange={setCreateOpen} />
    </>
  );
}
