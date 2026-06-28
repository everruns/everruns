/**
 * Shared OSS onboarding step list.
 *
 * Decision: the step list is data, not hardcoded inside shell usages. OSS
 * self-host has no email-verification step (any authenticated user may create
 * their first org), so the OSS list is three steps. SaaS supplies its own list
 * that prepends a "Verify" step — so consumers import a list and pass it to
 * `OnboardingShell` rather than hardcoding the steps inline.
 */
import type { OnboardingStep } from "./onboarding-stepper";

export const OSS_ONBOARDING_STEPS: OnboardingStep[] = [
  { key: "organisation", label: "Organisation" },
  { key: "configure", label: "Configure" },
  { key: "done", label: "Done" },
];
