"use client";

import { usePageTitle } from "@/hooks";
import { DeclarativeCapabilityForm } from "@/components/capabilities/declarative-capability-form";

export default function NewDeclarativeCapabilityPage() {
  usePageTitle("New Declarative Capability");
  return <DeclarativeCapabilityForm mode="create" />;
}
