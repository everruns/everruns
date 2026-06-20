"use client";

import { ZeroOrgOnboarding } from "@/components/onboarding/zero-org-onboarding";
import { usePageTitle } from "@/hooks";

export default function OnboardingPage() {
  usePageTitle("Get started");
  return <ZeroOrgOnboarding />;
}
