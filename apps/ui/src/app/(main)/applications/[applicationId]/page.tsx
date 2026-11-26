"use client";
import { use } from "react";
import { useApplication } from "@/hooks/use-applications";
import { ApplicationEditor } from "@/components/applications/application-editor";
import { useUiTranslations } from "@/providers/locale-provider";
export default function Page({ params }: { params: Promise<{ applicationId: string }> }) {
  const { applicationId } = use(params);
  const query = useApplication(applicationId);
  const ui = useUiTranslations();
  if (query.isLoading)
    return (
      <p className="p-8" role="status">
        {ui("Loading...")}
      </p>
    );
  if (!query.data)
    return (
      <p className="p-8" role="alert">
        {ui("Could not load application")}
      </p>
    );
  return (
    <ApplicationEditor
      key={query.data.id + ":" + query.data.version}
      application={query.data}
      onSaved={() => query.refetch()}
    />
  );
}
