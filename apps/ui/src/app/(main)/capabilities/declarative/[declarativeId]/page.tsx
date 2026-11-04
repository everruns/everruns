"use client";

import { useUiTranslations } from "@/providers/locale-provider";
import { use } from "react";
import { useDeclarativeCapability, usePageTitle } from "@/hooks";
import { Skeleton } from "@/components/ui/skeleton";
import { ResourceNotFound } from "@/components/resource-not-found";
import { DeclarativeCapabilityForm } from "@/components/capabilities/declarative-capability-form";

export default function EditDeclarativeCapabilityPage({
  params,
}: {
  params: Promise<{ declarativeId: string }>;
}) {
  const ui = useUiTranslations();
  const { declarativeId } = use(params);
  const { data: capability, isLoading, error } = useDeclarativeCapability(declarativeId);
  usePageTitle(capability?.display_name ?? capability?.name ?? null, ui("Edit Capability"));

  if (isLoading) {
    return (
      <div className="container mx-auto p-6">
        <Skeleton className="h-4 w-24 mb-6" />
        <Skeleton className="h-8 w-1/3 mb-4" />
        <Skeleton className="h-64 w-full" />
      </div>
    );
  }

  if (error || !capability) {
    return (
      <ResourceNotFound
        title={error ? ui("Capability unavailable") : ui("Capability not found")}
        description={
          error
            ? ui("The capability could not be loaded: {value0}", { value0: error.message })
            : ui(
                "This declarative capability may have been deleted or moved to another organization.",
              )
        }
        backHref="/capabilities"
        backLabel="Back to capabilities"
        resourceId={declarativeId}
      />
    );
  }

  return (
    <DeclarativeCapabilityForm
      mode="edit"
      id={capability.id}
      initialDefinition={capability.definition}
    />
  );
}
