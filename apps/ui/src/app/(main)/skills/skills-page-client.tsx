"use client";

import { useLocale as useDisplayLocale, useUiTranslations } from "@/providers/locale-provider";

import { useTextPageFormDraft } from "@/components/chat-mode/use-text-page-form-draft";

import { useChatPageFilters } from "@/components/chat-mode/page-view-state";

import { EntityStatus } from "@/components/ui/entity-status";
import { useMemo, useState } from "react";
import { Button } from "@/components/ui/button";
import { EntityCard, EntityCardDescription } from "@/components/ui/entity-card";
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
import { getDisplayName, getEntityNameClassName, isArchivedStatus } from "@/lib/entity-lifecycle";
import {
  PageContainer,
  PageBreadcrumb,
  PageMasthead,
  IconTile,
  PageControlStrip,
  SectionTabs,
  EmptyState,
  PageMain,
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
  const { displayLocale } = useDisplayLocale();
  const ui = useUiTranslations();
  const updateSkill = useUpdateSkill(skill.id);
  const isArchived = skill.status === "archived";
  const isDeleted = skill.status === "deleted";

  return (
    <EntityCard
      variant="row"
      icon={
        <IconTile size="md" icon={skill.source_type === "archive" ? <Archive /> : <FileText />} />
      }
      title={getDisplayName(skill, displayLocale)}
      href={`/skills/${skill.id}`}
      titleClassName={getEntityNameClassName(skill.status)}
      inlineBadges={
        <>
          <EntityStatus status={skill.status} />
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
              {updateSkill.isPending ? ui("Archiving...") : ui("Archive")}
            </Button>
          )}
          {isArchived && canDestroy && (
            <Button variant="destructive" size="sm" onClick={() => onDelete(skill)}>
              <Trash2 className="h-4 w-4 mr-1" />
              {ui("Delete")}
            </Button>
          )}
        </>
      }
    >
      <div className="mt-1 space-y-1.5 text-sm">
        {skill.description && <EntityCardDescription>{skill.description}</EntityCardDescription>}
        <div className="flex flex-wrap items-center gap-x-4 gap-y-1 text-xs text-muted-foreground">
          {skill.license && (
            <span>
              {ui("License:")} {skill.license}
            </span>
          )}
          {skill.allowed_tools && (
            <span>
              {ui("Tools:")} {skill.allowed_tools}
            </span>
          )}
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
  const ui = useUiTranslations();
  const [skillMd, setSkillMd] = useState(
    `---\nname: my-skill\ndescription: Describe what this skill does.\n---\n\n# My Skill\n\nInstructions here.\n`,
  );

  const createSkill = useCreateSkill();
  const chatDraft = useTextPageFormDraft({
    fields: ["skill_md"],
    values: { skill_md: skillMd },
    entityType: "skill",
    label: ui("New Skill"),
    baseline: "new",
    dirty: !!skillMd,
    writable: !createSkill.isPending,
    enabled: open,
    priority: 30,
    apply: (values) => setSkillMd(values.skill_md),
  });

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (chatDraft.blocked) return;
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
          <DialogTitle>{ui("Add Skill")}</DialogTitle>
          <DialogDescription>
            {ui(
              "Create a new skill from SKILL.md content. Use YAML frontmatter for metadata and markdown for instructions.",
            )}
          </DialogDescription>
        </DialogHeader>
        <form onSubmit={handleSubmit} className="space-y-4">
          {chatDraft.notice}
          <div className="space-y-2">
            <Label htmlFor="skill-md">{ui("SKILL.md")}</Label>
            <Textarea
              id="skill-md"
              value={skillMd}
              onChange={(e: React.ChangeEvent<HTMLTextAreaElement>) => setSkillMd(e.target.value)}
              placeholder={ui("---\nname: my-skill\ndescription: ...\n---\n\nInstructions...")}
              rows={14}
              className="font-mono text-sm"
              required
            />
          </div>
          <DialogFooter>
            <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
              {ui("Cancel")}
            </Button>
            <Button type="submit" disabled={createSkill.isPending || !skillMd.trim()}>
              {createSkill.isPending ? ui("Creating...") : ui("Create Skill")}
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
  const ui = useUiTranslations();
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
          <DialogTitle>{ui("Upload Skill Archive")}</DialogTitle>
          <DialogDescription>
            {ui(
              "Upload a ZIP archive containing a SKILL.md file and optional scripts, references, and assets.",
            )}
          </DialogDescription>
        </DialogHeader>
        <form onSubmit={handleSubmit} className="space-y-4">
          <div className="space-y-2">
            <Label htmlFor="skill-file">{ui("ZIP Archive")}</Label>
            <input
              id="skill-file"
              type="file"
              accept=".zip"
              aria-label={ui("Skill ZIP archive")}
              onChange={(e) => setFile(e.target.files?.[0] ?? null)}
              className="block w-full text-sm text-muted-foreground file:mr-4 file:py-2 file:px-4 file:border-0 file:text-sm file:font-medium file:bg-primary file:text-primary-foreground hover:file:bg-primary/90"
            />
            <p className="text-xs text-muted-foreground">{ui("Maximum archive size: 10 MB")}</p>
          </div>
          <DialogFooter>
            <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
              {ui("Cancel")}
            </Button>
            <Button type="submit" disabled={uploadSkill.isPending || !file}>
              {uploadSkill.isPending ? ui("Uploading...") : ui("Upload")}
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
  const ui = useUiTranslations();
  const [statusTab, setStatusTab] = useState<StatusTab>("active");
  const [search, setSearch] = useState("");
  useChatPageFilters({ status: statusTab, search: search });
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

  const handleDeleteSkill = async () => {
    if (!pendingDeleteSkill) return;
    await destroySkillMutation.mutateAsync(pendingDeleteSkill.id);
    setPendingDeleteSkill(null);
  };

  const statusItems = [
    { value: "all" as const, label: ui("All") },
    { value: "active" as const, label: ui("Active") },
    { value: "archived" as const, label: ui("Archived") },
  ];

  return (
    <PageContainer>
      <PageBreadcrumb items={[{ label: ui("Skills") }]} />

      <PageMasthead
        icon={<SkillsIcon />}
        title={ui("Skills")}
        description={ui("Reusable Agent Skills: SKILL.md instructions and optional bundled files.")}
        actions={
          <>
            <Button variant="outline" onClick={() => setUploadSkillOpen(true)}>
              <Upload className="size-4" />
              {ui("Upload ZIP")}
            </Button>
            <Button variant="accent" onClick={() => setAddSkillOpen(true)}>
              <Plus className="size-4" />
              {ui("Add Skill")}
            </Button>
          </>
        }
      />

      <PageControlStrip className="flex flex-wrap items-center gap-3">
        <SearchInput
          containerClassName="w-64"
          placeholder={ui("Search skills…")}
          value={search}
          onChange={(e) => setSearch(e.target.value)}
          aria-label={ui("Search skills")}
        />
        <div className="flex-1" />
        <SectionTabs
          value={statusTab}
          onValueChange={(v) => setStatusTab(v as StatusTab)}
          items={statusItems}
        />
      </PageControlStrip>

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
                  ? ui("No skills match your filters.")
                  : ui("No skills yet")
              }
              action={
                !search.trim() &&
                statusTab === "active" && (
                  <>
                    <Button variant="outline" onClick={() => setUploadSkillOpen(true)}>
                      <Upload className="size-4" />
                      {ui("Upload ZIP")}
                    </Button>
                    <Button variant="accent" onClick={() => setAddSkillOpen(true)}>
                      <Plus className="size-4" />
                      {ui("Add Skill")}
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

      <AddSkillDialog open={addSkillOpen} onOpenChange={setAddSkillOpen} />
      <UploadSkillDialog open={uploadSkillOpen} onOpenChange={setUploadSkillOpen} />
      <Dialog
        open={pendingDeleteSkill !== null}
        onOpenChange={(open) => !open && setPendingDeleteSkill(null)}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{ui("Delete Skill")}</DialogTitle>
            <DialogDescription>
              {ui("Permanently delete the archived skill")}{" "}
              <span className="font-medium">{pendingDeleteSkill?.name}</span>
              {ui("? Existing references will render as deleted tombstones.")}
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" onClick={() => setPendingDeleteSkill(null)}>
              {ui("Cancel")}
            </Button>
            <Button
              variant="destructive"
              onClick={handleDeleteSkill}
              disabled={destroySkillMutation.isPending}
            >
              <Box className="h-4 w-4 mr-1" />
              {destroySkillMutation.isPending ? ui("Deleting...") : ui("Delete Skill")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </PageContainer>
  );
}
