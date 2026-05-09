"use client";

import { use } from "react";
import Link from "next/link";
import { ArrowLeft, Archive, FileText } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Skeleton } from "@/components/ui/skeleton";
import { ResourceNotFound } from "@/components/resource-not-found";
import { SkillDetailView } from "@/components/skills/skill-detail-view";
import { usePageTitle } from "@/hooks";
import { useSkill, useSkillContent, useSkillsUsage } from "@/hooks/use-skills";
import {
  getDisplayName,
  getEntityNameClassName,
  getEntityStatusBadgeVariant,
} from "@/lib/entity-lifecycle";

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

  return (
    <div className="container mx-auto p-6">
      <Link
        href="/skills"
        className="mb-6 inline-flex items-center text-sm text-muted-foreground hover:text-foreground"
      >
        <ArrowLeft className="mr-2 h-4 w-4" />
        Back to Skills
      </Link>

      <header className="mb-6 flex flex-col gap-4 lg:flex-row lg:items-start lg:justify-between">
        <div className="min-w-0">
          <h1 className="flex min-w-0 flex-wrap items-center gap-2 text-2xl font-bold">
            {skill.source_type === "archive" ? (
              <Archive className="h-6 w-6 shrink-0 text-primary" />
            ) : (
              <FileText className="h-6 w-6 shrink-0 text-primary" />
            )}
            <span className={getEntityNameClassName(skill.status)}>{getDisplayName(skill)}</span>
            <Badge variant={getEntityStatusBadgeVariant(skill.status)}>{skill.status}</Badge>
          </h1>
          <p className="mt-2 max-w-3xl text-sm leading-6 text-muted-foreground">
            {skill.description}
          </p>
        </div>
      </header>

      <SkillDetailView
        skill={skill}
        usage={usage?.[skill.id]}
        content={content}
        contentLoading={contentLoading}
        contentError={contentError}
      />
    </div>
  );
}
