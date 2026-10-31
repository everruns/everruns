"use client";

import { useUiTranslations } from "@/providers/locale-provider";
import { usePageTitle } from "@/hooks";
import { DeclarativeCapabilityForm } from "@/components/capabilities/declarative-capability-form";

export default function NewDeclarativeCapabilityPage() {
  const ui = useUiTranslations();
  usePageTitle(ui("New declarative capability"));
  return <DeclarativeCapabilityForm mode="create" />;
}
