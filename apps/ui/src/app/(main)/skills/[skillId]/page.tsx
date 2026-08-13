"use client";

import { use } from "react";
import { Archive, FileText } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Skeleton } from "@/components/ui/skeleton";
import { ResourceNotFound } from "@/components/resource-not-found";
import { SkillDetailView } from "@/components/skills/skill-detail-view";
import { SkillUsageRow } from "@/components/skills/skill-usage-row";
import { usePageTitle } from "@/hooks";
import { useSkill, useSkillContent, useSkillsUsage } from "@/hooks/use-skills";
import {
  getDisplayName,
  getEntityNameClassName,
  getEntityStatusBadgeVariant,
} from "@/lib/entity-lifecycle";
import {
  PageContainer,
  PageBreadcrumb,
  PageMasthead,
  PageColumns,
  PageMain,
  PageRail,
  RailSection,
  PageFooter,
  BackLink,
} from "@/components/layout";

export default function SkillDetailPage({ params }: { params: Promise<{ skillId: string }> }) {
  const { skillId } = use(params);
  const { data: skill, isLoading: skillLoading } = useSkill(skillId);
  const {
    data: content,
    isLoading: contentLoading,
    error: contentError,
  } = useSkillContent(skillId);
  const { data: usage } = useSkillsUsage();
  usePageTitle(skill ? getDisplayName(skill) : null, "Skill");

  if (skillLoading) {
    return (
      <div className="container mx-auto p-6">
        <Skeleton className="mb-6 h-5 w-36" />
        <Skeleton className="mb-4 h-8 w-1/3" />
        <Skeleton className="mb-8 h-4 w-2/3" />
        <Skeleton className="h-72 w-full" />
      </div>
    );
  }

  if (!skill) {
    return (
      <ResourceNotFound
        title="Skill not found"
        description="This skill may have been deleted, moved to another organization, or the URL may be wrong."
        backHref="/skills"
        backLabel="Back to skills"
        resourceId={skillId}
      />
    );
  }

  const skillUsage = usage?.[skill.id];

  return (
    <PageContainer>
      <PageBreadcrumb
        items={[{ label: "Skills", href: "/skills" }, { label: getDisplayName(skill) }]}
      />

      <PageMasthead
        icon={skill.source_type === "archive" ? <Archive /> : <FileText />}
        entityId={skill.id}
        title={
          <span className={getEntityNameClassName(skill.status)}>{getDisplayName(skill)}</span>
        }
        badges={
          <>
            <Badge variant={getEntityStatusBadgeVariant(skill.status)}>{skill.status}</Badge>
            <Badge variant="secondary" className="text-xs">
              {skill.source_type}
            </Badge>
          </>
        }
        description={skill.description || undefined}
        meta={
          <>
            <span>
              Version <span className="text-primary">v{skill.version}</span>
            </span>
            {skill.license && (
              <span>
                License <span className="text-foreground">{skill.license}</span>
              </span>
            )}
            <span>
              Created{" "}
              <span className="text-foreground">
                {new Date(skill.created_at).toLocaleDateString()}
              </span>
            </span>
          </>
        }
      />

      <PageColumns>
        <PageMain>
          <SkillDetailView
            skill={skill}
            usage={skillUsage}
            content={content}
            contentLoading={contentLoading}
            contentError={contentError}
          />
        </PageMain>

        <PageRail>
          <RailSection label="Used by">
            <SkillUsageRow usage={skillUsage} />
          </RailSection>

          <RailSection label="Details">
            <dl className="flex flex-col gap-2 text-[13px]">
              <div className="flex items-center justify-between gap-2">
                <dt className="text-muted-foreground">Source</dt>
                <dd className="text-foreground">{skill.source_type}</dd>
              </div>
              <div className="flex items-center justify-between gap-2">
                <dt className="text-muted-foreground">Version</dt>
                <dd className="text-foreground">v{skill.version}</dd>
              </div>
              <div className="flex items-center justify-between gap-2">
                <dt className="text-muted-foreground">License</dt>
                <dd className="text-foreground">{skill.license ?? "-"}</dd>
              </div>
              <div className="flex items-center justify-between gap-2">
                <dt className="text-muted-foreground">Compatibility</dt>
                <dd className="text-foreground">{skill.compatibility ?? "-"}</dd>
              </div>
              <div className="flex flex-col gap-0.5">
                <dt className="text-muted-foreground">Allowed tools</dt>
                <dd className="break-words text-foreground">{skill.allowed_tools ?? "-"}</dd>
              </div>
              <div className="flex items-center justify-between gap-2">
                <dt className="text-muted-foreground">Updated</dt>
                <dd className="text-foreground">
                  {new Date(skill.updated_at).toLocaleDateString()}
                </dd>
              </div>
            </dl>
          </RailSection>
        </PageRail>
      </PageColumns>

      <PageFooter>
        <BackLink href="/skills">Back to Skills</BackLink>
      </PageFooter>
    </PageContainer>
  );
}
