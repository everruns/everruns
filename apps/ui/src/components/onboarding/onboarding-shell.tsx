/**
 * Presentational onboarding shell — the shared chrome that onboarding steps
 * (verify / create-org / configure / done) render inside.
 *
 * Composes the approved design: a header bar (logo + Everruns wordmark on the
 * left, optional step label on the right), an optional stepper row beneath it,
 * and a two-column body — the left pane holds `children`, the right pane is the
 * navy brand panel. On small screens the brand panel hides and the layout
 * collapses to a single column (the stepper labels stay readable; circles wrap
 * as needed).
 *
 * The shell is the inner framed surface (white `card`, 1px borders, no shadow),
 * matching the design frames. Route layouts (e.g. `(auth)/layout.tsx`) provide
 * the textured `bg-background bg-brand-dots` page background.
 *
 * Pure/presentational — no data fetching, no routing.
 */
import type { ReactNode } from "react";
import Image from "next/image";
import { OnboardingBrandPanel, type OnboardingBrandPanelProps } from "./onboarding-brand-panel";
import { OnboardingStepper, type OnboardingStep } from "./onboarding-stepper";

export interface OnboardingShellProps {
  /** Right-aligned header text, e.g. "Step 2 of 4". */
  stepLabel?: string;
  /** Stepper steps. Omit to hide the stepper row. */
  steps?: OnboardingStep[];
  /** Zero-based index of the active step (used when `steps` is provided). */
  currentIndex?: number;
  /** Brand panel content for the right column. */
  brand: OnboardingBrandPanelProps;
  /** Left content pane. */
  children: ReactNode;
}

export function OnboardingShell({
  stepLabel,
  steps,
  currentIndex = 0,
  brand,
  children,
}: OnboardingShellProps) {
  return (
    <div className="flex w-full max-w-[1200px] flex-col overflow-hidden border bg-card text-card-foreground">
      {/* Header bar */}
      <div className="flex h-[60px] flex-shrink-0 items-center justify-between border-b px-6 sm:px-10">
        <div className="flex items-center gap-[10px]">
          <Image src="/logo.svg" alt="Everruns" width={26} height={26} />
          <span className="text-[15px] font-semibold tracking-[-0.02em]">Everruns</span>
        </div>
        {stepLabel && (
          <div className="font-mono text-[11px] text-muted-foreground">{stepLabel}</div>
        )}
      </div>

      {/* Stepper row */}
      {steps && steps.length > 0 && (
        <div className="flex flex-shrink-0 items-center overflow-x-auto border-b px-6 py-5 sm:h-[72px] sm:px-10 sm:py-0">
          <OnboardingStepper steps={steps} currentIndex={currentIndex} />
        </div>
      )}

      {/* Body */}
      <div className="flex flex-1 flex-col lg:flex-row">
        <div className="flex flex-1 flex-col justify-center px-6 py-10 sm:px-12 lg:px-16">
          {children}
        </div>
        <OnboardingBrandPanel {...brand} />
      </div>
    </div>
  );
}
