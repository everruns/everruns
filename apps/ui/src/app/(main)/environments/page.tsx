"use client";

import { Box, Container, Plus } from "lucide-react";
import { useEnvironments, usePageTitle } from "@/hooks";
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

export default function EnvironmentsPage() {
  usePageTitle("Environments");
  const { data, isLoading, error } = useEnvironments();
  return (
    <PageContainer>
      <PageBreadcrumb items={[{ label: "Environments" }]} />
      <PageMasthead
        icon={<Container />}
        title="Environments"
        description="Reusable, versioned configuration for the primary Sandbox each Session receives."
        actions={
          <LinkButton variant="accent" href="/environments/new">
            <Plus className="size-4" /> New Environment
          </LinkButton>
        }
      />
      <PageMain>
        <QueryStateWrapper
          data={data}
          isLoading={isLoading}
          error={error}
          errorMessagePrefix="Failed to load environments"
          emptyState={
            <EmptyState
              icon={<Container />}
              title="No environments"
              action={<LinkButton href="/environments/new">Create an Environment</LinkButton>}
            />
          }
        >
          {(items) => (
            <div className="grid gap-4 xl:grid-cols-2">
              {items.map((environment) => {
                const target = environment.current_revision.profile.target;
                return (
                  <EntityCard
                    key={environment.id}
                    href={`/environments/${environment.id}`}
                    icon={<Box className="size-5 text-muted-foreground" />}
                    title={environment.display_name}
                    subtitle={environment.name}
                    headerActions={
                      <div className="flex gap-2">
                        {environment.is_managed ? <Badge>Managed</Badge> : null}
                        <Badge variant="outline">
                          Revision {environment.current_revision.revision}
                        </Badge>
                      </div>
                    }
                    footer={
                      <EntityCardFooter
                        meta={`${target.provider || target.kind} · ${environment.current_revision.profile.durability}`}
                      />
                    }
                  >
                    <EntityCardDescription>
                      {environment.description || "Reusable Sandbox configuration"}
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
