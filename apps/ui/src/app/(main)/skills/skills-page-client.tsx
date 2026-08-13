"use client";

import { useMemo, useState } from "react";
import { Button } from "@/components/ui/button";
import { EntityCard } from "@/components/ui/entity-card";
import { Badge } from "@/components/ui/badge";
import { Skeleton } from "@/components/ui/skeleton";
import { QueryStateWrapper } from "@/components/query-state-wrapper";
import { SearchInput } from "@/components/ui/search-input";
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
  useSkillsUsage,
  useUpdateSkill,
  useUploadSkill,
} from "@/hooks/use-skills";
import { usePolicies } from "@/hooks/use-policies";
import { Plus, Trash2, Upload, FileText, Archive, Box } from "lucide-react";
import { registryDomainIcons } from "@/lib/registry-navigation";
import type { Skill, SkillUsage } from "@/lib/api/types";
import {
  getDisplayName,
  getEntityNameClassName,
  getEntityStatusBadgeVariant,
  isArchivedStatus,
} from "@/lib/entity-lifecycle";
import { pluralize } from "@/lib/formatting";
import { cn } from "@/lib/utils";
import {
  PageContainer,
  PageBreadcrumb,
  PageMasthead,
  IconTile,
  PageControlStrip,
  SectionTabs,
  EmptyState,
  PageColumns,
  PageMain,
  PageRail,
  RailSection,
  PageFooter,
} from "@/components/layout";
import { SkillUsageRow } from "@/components/skills/skill-usage-row";

type StatusTab = "all" | "active" | "archived";

const SkillsIcon = registryDomainIcons.skills;

function SkillRow({
  skill,
  usage,
  canDestroy,
  onDelete,
}: {
  skill: Skill;
  usage: SkillUsage | undefined;
  canDestroy: boolean;
  onDelete: (skill: Skill) => void;
}) {
  const updateSkill = useUpdateSkill(skill.id);
  const isArchived = skill.status === "archived";
  const isDeleted = skill.status === "deleted";

  return (
    <EntityCard
      variant="row"
      icon={
        <IconTile size="md" icon={skill.source_type === "archive" ? <Archive /> : <FileText />} />
      }
      title={getDisplayName(skill)}
      href={`/skills/${skill.id}`}
      titleClassName={getEntityNameClassName(skill.status)}
      copyValue={skill.id}
      inlineBadges={
        <>
          <Badge variant={getEntityStatusBadgeVariant(skill.status)}>{skill.status}</Badge>
          <Badge variant="secondary" className="text-xs">
            {skill.source_type}
          </Badge>
          <span className="text-xs text-muted-foreground">v{skill.version}</span>
        </>
      }
      headerActions={
        <>
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
        </>
      }
    >
      <div className="mt-1 space-y-1.5 text-sm">
        {skill.description && (
          <p className="line-clamp-2 text-muted-foreground">{skill.description}</p>
        )}
        <div className="flex flex-wrap items-center gap-x-4 gap-y-1 text-xs text-muted-foreground">
          {skill.license && <span>License: {skill.license}</span>}
          {skill.allowed_tools && <span>Tools: {skill.allowed_tools}</span>}
        </div>
        <SkillUsageRow usage={usage} />
      </div>
    </EntityCard>
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
              aria-label="Skill ZIP archive"
              onChange={(e) => setFile(e.target.files?.[0] ?? null)}
              className="block w-full text-sm text-muted-foreground file:mr-4 file:py-2 file:px-4 file:border-0 file:text-sm file:font-medium file:bg-primary file:text-primary-foreground hover:file:bg-primary/90"
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

function SkillRowSkeleton() {
  return (
    <div className="flex items-start justify-between border p-3">
      <div className="flex flex-1 items-start gap-3">
        <Skeleton className="h-8 w-8" />
        <div className="space-y-2">
          <Skeleton className="h-5 w-40" />
          <Skeleton className="h-4 w-64" />
        </div>
      </div>
      <Skeleton className="h-8 w-24" />
    </div>
  );
}

export default function SkillsPageClient() {
  const [statusTab, setStatusTab] = useState<StatusTab>("active");
  const [search, setSearch] = useState("");
  // Active tab fetches active-only (matching the SSR seed); other tabs include archived.
  const includeArchived = statusTab !== "active";
  const { data: skills, isLoading, error } = useSkills({ includeArchived });
  const { data: usage } = useSkillsUsage();
  const destroySkillMutation = useDestroySkill();
  const { can: canPolicies } = usePolicies("skills");
  const canDestroy = canPolicies("skill.dangerous");

  const [addSkillOpen, setAddSkillOpen] = useState(false);
  const [uploadSkillOpen, setUploadSkillOpen] = useState(false);
  const [pendingDeleteSkill, setPendingDeleteSkill] = useState<Skill | null>(null);

  const counts = useMemo(() => {
    const list = skills ?? [];
    const archived = list.filter((s) => isArchivedStatus(s.status)).length;
    return { all: list.length, active: list.length - archived, archived };
  }, [skills]);

  const filteredSkills = useMemo(() => {
    const query = search.trim().toLowerCase();
    return (skills ?? []).filter((s) => {
      if (statusTab === "active" && isArchivedStatus(s.status)) return false;
      if (statusTab === "archived" && !isArchivedStatus(s.status)) return false;
      if (!query) return true;
      return (
        s.name.toLowerCase().includes(query) ||
        s.description.toLowerCase().includes(query) ||
        (s.allowed_tools?.toLowerCase().includes(query) ?? false)
      );
    });
  }, [skills, search, statusTab]);

  // Source-type usage counts across the fetched skills, for the rail facet.
  const sourceFacets = useMemo(() => {
    const tally = new Map<string, number>();
    for (const skill of skills ?? []) {
      tally.set(skill.source_type, (tally.get(skill.source_type) ?? 0) + 1);
    }
    return [...tally.entries()].sort((a, b) => b[1] - a[1]);
  }, [skills]);

  const handleDeleteSkill = async () => {
    if (!pendingDeleteSkill) return;
    await destroySkillMutation.mutateAsync(pendingDeleteSkill.id);
    setPendingDeleteSkill(null);
  };

  const statusItems = [
    { value: "all" as const, label: "All" },
    { value: "active" as const, label: "Active" },
    { value: "archived" as const, label: "Archived" },
  ];

  return (
    <PageContainer>
      <PageBreadcrumb items={[{ label: "Skills" }]} />

      <PageMasthead
        icon={<SkillsIcon />}
        title="Skills"
        badges={
          <Badge variant="outline" className="font-mono">
            {counts.all}
          </Badge>
        }
        description="Reusable Agent Skills — SKILL.md instructions and optional bundled files."
        meta={
          <>
            <span>{counts.active} active</span>
            <span>{counts.archived} archived</span>
          </>
        }
        actions={
          <>
            <Button variant="outline" onClick={() => setUploadSkillOpen(true)}>
              <Upload className="size-4" />
              Upload ZIP
            </Button>
            <Button variant="accent" onClick={() => setAddSkillOpen(true)}>
              <Plus className="size-4" />
              Add Skill
            </Button>
          </>
        }
      />

      <PageControlStrip className="flex flex-wrap items-center gap-3">
        <SearchInput
          containerClassName="w-64"
          placeholder="Search skills…"
          value={search}
          onChange={(e) => setSearch(e.target.value)}
          aria-label="Search skills"
        />
        <div className="flex-1" />
        <SectionTabs
          value={statusTab}
          onValueChange={(v) => setStatusTab(v as StatusTab)}
          items={statusItems}
        />
      </PageControlStrip>

      <PageColumns>
        <PageMain>
          <QueryStateWrapper
            isLoading={isLoading}
            error={error}
            data={filteredSkills}
            errorMessagePrefix="Failed to load skills"
            loadingSkeleton={
              <div className="flex flex-col gap-3">
                {[...Array(6)].map((_, i) => (
                  <SkillRowSkeleton key={i} />
                ))}
              </div>
            }
            emptyState={
              <EmptyState
                icon={<SkillsIcon />}
                title={
                  search.trim() || statusTab !== "active"
                    ? "No skills match your filters."
                    : "No skills yet"
                }
                action={
                  !search.trim() &&
                  statusTab === "active" && (
                    <>
                      <Button variant="outline" onClick={() => setUploadSkillOpen(true)}>
                        <Upload className="size-4" />
                        Upload ZIP
                      </Button>
                      <Button variant="accent" onClick={() => setAddSkillOpen(true)}>
                        <Plus className="size-4" />
                        Add Skill
                      </Button>
                    </>
                  )
                }
              />
            }
          >
            {(items) => (
              <div className="flex flex-col gap-3">
                {items.map((skill) => (
                  <SkillRow
                    key={skill.id}
                    skill={skill}
                    usage={usage?.[skill.id]}
                    canDestroy={canDestroy}
                    onDelete={setPendingDeleteSkill}
                  />
                ))}
              </div>
            )}
          </QueryStateWrapper>
        </PageMain>

        <PageRail>
          <RailSection label="Status">
            <div className="flex flex-col gap-1.5 text-[13px]">
              {statusItems.map((item) => (
                <button
                  key={item.value}
                  type="button"
                  onClick={() => setStatusTab(item.value)}
                  className={cn(
                    "flex items-center justify-between transition-colors hover:text-foreground",
                    statusTab === item.value ? "text-foreground" : "text-muted-foreground",
                  )}
                >
                  <span>{item.label}</span>
                  <span className="text-muted-foreground">{counts[item.value]}</span>
                </button>
              ))}
            </div>
          </RailSection>

          {sourceFacets.length > 0 && (
            <RailSection label="Source">
              <div className="flex flex-col gap-1.5 text-[13px] text-muted-foreground">
                {sourceFacets.map(([source, count]) => (
                  <div key={source} className="flex items-center justify-between">
                    <span className="inline-flex items-center gap-1.5">
                      {source === "archive" ? (
                        <Archive className="size-3.5" />
                      ) : (
                        <FileText className="size-3.5" />
                      )}
                      {source}
                    </span>
                    <span>{count}</span>
                  </div>
                ))}
              </div>
            </RailSection>
          )}
        </PageRail>
      </PageColumns>

      <PageFooter>
        <span>
          Showing {filteredSkills.length} of {counts.all} {pluralize(counts.all, "skill")}
        </span>
        {counts.archived > 0 && statusTab !== "archived" && (
          <button
            type="button"
            onClick={() => setStatusTab("archived")}
            className="text-primary transition-colors hover:underline"
          >
            View archived →
          </button>
        )}
      </PageFooter>

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
    </PageContainer>
  );
}
