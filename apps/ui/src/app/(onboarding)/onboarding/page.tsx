"use client";

import { useUiTranslations } from "@/providers/locale-provider";
import { ZeroOrgOnboarding } from "@/components/onboarding/zero-org-onboarding";
import { usePageTitle } from "@/hooks";

export default function OnboardingPage() {
  const ui = useUiTranslations();
  usePageTitle(ui("Get started"));
  return <ZeroOrgOnboarding />;
}
