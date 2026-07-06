"use client";

/**
 * Onboarding-arc context — lets an embedding wrapper (e.g. SaaS) extend the
 * onboarding stepper with extra leading steps without forking the OSS
 * surfaces that render it.
 *
 * Decision: the OSS pages (`ZeroOrgOnboarding`, org setup) index their
 * position in the arc relative to the OSS step list. A wrapper that prepends
 * steps (SaaS prepends "Verify") provides the full list plus the offset of
 * the first OSS step within it; the OSS surfaces then render the wrapper's
 * stepper and "Step N of M" labels without any page-level overrides. With no
 * provider mounted, everything falls back to the plain OSS 3-step arc.
 */
import { createContext, useContext, type ReactNode } from "react";
import type { OnboardingStep } from "./onboarding-stepper";
import { OSS_ONBOARDING_STEPS } from "./steps";

export interface OnboardingArcConfig {
  /** Full step list for the arc, in render order. */
  steps: OnboardingStep[];
  /**
   * Index of the first OSS step ("Organisation") within `steps`. OSS default
   * is 0; SaaS passes 1 because it prepends a "Verify" step.
   */
  ossStepOffset: number;
}

const DEFAULT_ARC: OnboardingArcConfig = {
  steps: OSS_ONBOARDING_STEPS,
  ossStepOffset: 0,
};

const OnboardingArcContext = createContext<OnboardingArcConfig>(DEFAULT_ARC);

export function OnboardingArcProvider({
  value,
  children,
}: {
  value: OnboardingArcConfig;
  children: ReactNode;
}) {
  return <OnboardingArcContext.Provider value={value}>{children}</OnboardingArcContext.Provider>;
}

export interface OnboardingArc extends OnboardingArcConfig {
  /** Absolute stepper index for a zero-based OSS step index. */
  stepIndex: (ossIndex: number) => number;
  /** Human "Step N of M" label for a zero-based OSS step index. */
  stepLabel: (ossIndex: number) => string;
}

export function useOnboardingArc(): OnboardingArc {
  const { steps, ossStepOffset } = useContext(OnboardingArcContext);
  return {
    steps,
    ossStepOffset,
    stepIndex: (ossIndex) => ossStepOffset + ossIndex,
    stepLabel: (ossIndex) => `Step ${ossStepOffset + ossIndex + 1} of ${steps.length}`,
  };
}
