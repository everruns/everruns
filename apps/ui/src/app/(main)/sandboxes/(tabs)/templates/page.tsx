"use client";

import { useUiTranslations } from "@/providers/locale-provider";
import { SandboxTemplateIcon } from "@/components/icons/facet-icons";
import { Box } from "lucide-react";
import { usePageTitle, useSandboxTemplates } from "@/hooks";
import { LinkButton } from "@/components/ui/button";
import { EntityCard, EntityCardDescription, EntityCardFooter } from "@/components/ui/entity-card";
import { Badge } from "@/components/ui/badge";
import { QueryStateWrapper } from "@/components/query-state-wrapper";
import { EmptyState, PageMain } from "@/components/layout";

// Templates tab. The page header, tabs and "New Sandbox Template" action come
// from the shell (../sandboxes-shell.tsx).
export default function SandboxTemplatesPage() {
  const ui = useUiTranslations();
  usePageTitle(ui("Templates"), ui("Sandboxes"));
  const { data, isLoading, error } = useSandboxTemplates();
  return (
    <PageMain>
      <QueryStateWrapper
        data={data}
        isLoading={isLoading}
        error={error}
        errorMessagePrefix="Failed to load Sandbox Templates"
        emptyState={
          <EmptyState
            icon={<SandboxTemplateIcon />}
            title={ui("No Sandbox Templates")}
            action={
              <LinkButton href="/sandboxes/templates/new">
                {ui("Create a Sandbox Template")}
              </LinkButton>
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
                  href={`/sandboxes/templates/${template.id}`}
                  icon={<Box className="size-5 text-muted-foreground" />}
                  title={template.display_name}
                  subtitle={template.name}
                  headerActions={
                    <div className="flex gap-2">
                      {template.is_managed ? <Badge>{ui("Managed")}</Badge> : null}
                      <Badge variant="outline">
                        {ui("Revision")} {template.current_revision.revision}
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
                    {template.description || ui("Reusable Sandbox configuration")}
                  </EntityCardDescription>
                </EntityCard>
              );
            })}
          </div>
        )}
      </QueryStateWrapper>
    </PageMain>
  );
}
