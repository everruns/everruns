"use client";

import { SandboxTemplateIcon } from "@/components/icons/facet-icons";
import { Box, Plus } from "lucide-react";
import { usePageTitle, useSandboxTemplates } from "@/hooks";
import { LinkButton } from "@/components/ui/button";
import { EntityCard, EntityCardDescription, EntityCardFooter } from "@/components/ui/entity-card";
import { Badge } from "@/components/ui/badge";
import { QueryStateWrapper } from "@/components/query-state-wrapper";
import {
  EmptyState,
  PageBreadcrumb,
  PageContainer,
  PageMain,
  PageMasthead,
} from "@/components/layout";

export default function SandboxTemplatesPage() {
  usePageTitle("Sandbox Templates");
  const { data, isLoading, error } = useSandboxTemplates();
  return (
    <PageContainer>
      <PageBreadcrumb items={[{ label: "Sandbox Templates" }]} />
      <PageMasthead
        icon={<SandboxTemplateIcon />}
        title="Sandbox Templates"
        description="Reusable, versioned configuration for the primary Sandbox each Session receives."
        actions={
          <LinkButton variant="accent" href="/sandbox-templates/new">
            <Plus className="size-4" /> New Sandbox Template
          </LinkButton>
        }
      />
      <PageMain>
        <QueryStateWrapper
          data={data}
          isLoading={isLoading}
          error={error}
          errorMessagePrefix="Failed to load Sandbox Templates"
          emptyState={
            <EmptyState
              icon={<SandboxTemplateIcon />}
              title="No Sandbox Templates"
              action={
                <LinkButton href="/sandbox-templates/new">Create a Sandbox Template</LinkButton>
              }
            />
          }
        >
          {(items) => (
            <div className="grid gap-4 xl:grid-cols-2">
              {items.map((template) => {
                const target = template.current_revision.spec.target;
                return (
                  <EntityCard
                    key={template.id}
                    href={`/sandbox-templates/${template.id}`}
                    icon={<Box className="size-5 text-muted-foreground" />}
                    title={template.display_name}
                    subtitle={template.name}
                    headerActions={
                      <div className="flex gap-2">
                        {template.is_managed ? <Badge>Managed</Badge> : null}
                        <Badge variant="outline">
                          Revision {template.current_revision.revision}
                        </Badge>
                      </div>
                    }
                    footer={
                      <EntityCardFooter
                        meta={`${target.provider || target.kind} · ${template.current_revision.spec.durability}`}
                      />
                    }
                  >
                    <EntityCardDescription>
                      {template.description || "Reusable Sandbox configuration"}
                    </EntityCardDescription>
                  </EntityCard>
                );
              })}
            </div>
          )}
        </QueryStateWrapper>
      </PageMain>
    </PageContainer>
  );
}
