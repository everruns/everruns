"use client";

import { useMemo, useState } from "react";
import type { ReactNode } from "react";
import { Archive, FileText, FolderTree, Info, ScrollText } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Label } from "@/components/ui/label";
import { Skeleton } from "@/components/ui/skeleton";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { InlineStreamdownMessage } from "@/components/chat/streamdown-message";
import { InitialFilesPreview } from "@/components/files/initial-files-preview";
import { SkillUsageRow } from "@/components/skills/skill-usage-row";
import { getEntityStatusBadgeVariant } from "@/lib/entity-lifecycle";
import type { InitialFile, Skill, SkillContent, SkillUsage } from "@/lib/api/types";

type SkillDetailTab = "overview" | "instructions" | "files";

interface SkillDetailViewProps {
  skill: Skill;
  usage: SkillUsage | undefined;
  content: SkillContent | undefined;
  contentLoading: boolean;
  contentError: unknown;
}

export function SkillDetailView({
  skill,
  usage,
  content,
  contentLoading,
  contentError,
}: SkillDetailViewProps) {
  const [activeTab, setActiveTab] = useState<SkillDetailTab>("overview");
  const split = useMemo(
    () => (content ? splitFrontmatter(content.skill_md) : { frontmatter: "", body: "" }),
    [content],
  );
  const skillFiles = useMemo(() => toReadonlySkillFiles(content), [content]);

  return (
    <Tabs
      value={activeTab}
      onValueChange={(value) => setActiveTab(value as SkillDetailTab)}
      className="space-y-6"
    >
      <TabsList>
        <TabsTrigger value="overview">
          <Info className="mr-2 h-4 w-4" />
          Overview
        </TabsTrigger>
        <TabsTrigger value="instructions">
          <ScrollText className="mr-2 h-4 w-4" />
          Instructions
        </TabsTrigger>
        <TabsTrigger value="files">
          <FolderTree className="mr-2 h-4 w-4" />
          Files
        </TabsTrigger>
      </TabsList>

      <TabsContent value="overview">
        <Card>
          <CardHeader>
            <CardTitle className="flex items-center gap-2 text-lg">
              {skill.source_type === "archive" ? (
                <Archive className="h-5 w-5 text-primary" />
              ) : (
                <FileText className="h-5 w-5 text-primary" />
              )}
              Details
            </CardTitle>
          </CardHeader>
          <CardContent>
            <section className="grid grid-cols-1 gap-x-6 gap-y-4 text-sm md:grid-cols-2">
              <DetailField label="Status">
                <Badge variant={getEntityStatusBadgeVariant(skill.status)}>{skill.status}</Badge>
              </DetailField>
              <DetailField label="Source">{skill.source_type}</DetailField>
              <DetailField label="Version">{skill.version}</DetailField>
              <DetailField label="License">{skill.license ?? "-"}</DetailField>
              <DetailField label="Compatibility">{skill.compatibility ?? "-"}</DetailField>
              <DetailField label="Allowed tools">{skill.allowed_tools ?? "-"}</DetailField>
              <DetailField label="Created">{formatDateTime(skill.created_at)}</DetailField>
              <DetailField label="Updated">{formatDateTime(skill.updated_at)}</DetailField>
              <div className="md:col-span-2">
                <Label className="text-muted-foreground">Used by</Label>
                <div className="mt-1">
                  <SkillUsageRow usage={usage} />
                </div>
              </div>
            </section>
          </CardContent>
        </Card>
      </TabsContent>

      <TabsContent value="instructions" className="space-y-6">
        <Card>
          <CardHeader>
            <CardTitle className="text-lg">Frontmatter</CardTitle>
          </CardHeader>
          <CardContent>
            {contentLoading && <Skeleton className="h-28 w-full" />}
            {contentError != null && (
              <p className="text-sm text-destructive">Failed to load skill content.</p>
            )}
            {content && (
              <pre className="max-h-[360px] overflow-auto bg-muted p-3 font-mono text-xs whitespace-pre-wrap break-all">
                {split.frontmatter || "(no frontmatter)"}
              </pre>
            )}
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <CardTitle className="text-lg">SKILL.md</CardTitle>
          </CardHeader>
          <CardContent>
            {contentLoading && <Skeleton className="h-64 w-full" />}
            {content && (
              <div className="min-h-[240px] overflow-x-auto bg-muted p-4 text-sm">
                {split.body.trim() ? (
                  <InlineStreamdownMessage>{split.body}</InlineStreamdownMessage>
                ) : (
                  <span className="text-muted-foreground">(empty)</span>
                )}
              </div>
            )}
          </CardContent>
        </Card>
      </TabsContent>

      <TabsContent value="files">
        {contentLoading ? (
          <Card>
            <CardContent className="p-6">
              <Skeleton className="h-[480px] w-full" />
            </CardContent>
          </Card>
        ) : contentError != null ? (
          <Card>
            <CardContent className="p-6">
              <p className="text-sm text-destructive">Failed to load skill files.</p>
            </CardContent>
          </Card>
        ) : (
          <InitialFilesPreview
            files={skillFiles}
            title="Skill Files"
            description="Read-only reconstructed skill files. Archive skills include extracted text files alongside SKILL.md."
          />
        )}
      </TabsContent>
    </Tabs>
  );
}

function DetailField({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div>
      <Label className="text-muted-foreground">{label}</Label>
      <div className="mt-1 break-words">{children}</div>
    </div>
  );
}

function splitFrontmatter(skillMd: string): { frontmatter: string; body: string } {
  const match = skillMd.match(/^---\r?\n([\s\S]*?)\r?\n---\r?\n?([\s\S]*)$/);
  if (!match) return { frontmatter: "", body: skillMd };
  return { frontmatter: match[1] ?? "", body: (match[2] ?? "").trimStart() };
}

function toReadonlySkillFiles(content: SkillContent | undefined): InitialFile[] {
  if (!content) return [];
  return [
    {
      path: "SKILL.md",
      content: content.skill_md,
      encoding: "text",
      is_readonly: true,
    },
    ...content.files.map((file) => ({
      path: file.path,
      content: file.content,
      encoding: "text" as const,
      is_readonly: true,
    })),
  ];
}

function formatDateTime(value: string): string {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;
  return date.toLocaleString();
}
