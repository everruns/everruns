/**
 * Data-driven horizontal stepper for the onboarding shell.
 *
 * Mirrors the stepper row from the approved onboarding design: numbered circles
 * with labels and connector lines. Steps before `currentIndex` are completed
 * (navy filled + check), the current step is active (gold ring/tint), and later
 * steps are pending (gray). Connector lines are navy up to the current step and
 * gray after. Works for any number of steps (3 or 4 in the design).
 *
 * Pure/presentational — no data fetching, no routing.
 */
import { Check } from "lucide-react";

export interface OnboardingStep {
  key: string;
  label: string;
}

export interface OnboardingStepperProps {
  steps: OnboardingStep[];
  /** Zero-based index of the active step. */
  currentIndex: number;
}

export function OnboardingStepper({ steps, currentIndex }: OnboardingStepperProps) {
  return (
    <div className="flex items-center">
      {steps.map((step, index) => {
        const completed = index < currentIndex;
        const active = index === currentIndex;
        const isLast = index === steps.length - 1;

        return (
          <div key={step.key} className="flex items-center">
            <div className="flex items-center gap-[10px]">
              <span
                className={[
                  "flex h-[26px] w-[26px] flex-shrink-0 items-center justify-center border font-mono text-xs",
                  completed
                    ? "border-primary bg-primary text-primary-foreground"
                    : active
                      ? "border-accent bg-accent/[0.14] text-accent-foreground"
                      : "border-border text-muted-foreground",
                ].join(" ")}
              >
                {completed ? (
                  <Check className="icon-sharp h-[13px] w-[13px]" strokeWidth={2.5} />
                ) : (
                  index + 1
                )}
              </span>
              <span
                className={[
                  "text-[13px]",
                  active
                    ? "font-semibold text-foreground"
                    : completed
                      ? "font-medium text-muted-foreground"
                      : "font-medium text-muted-foreground/70",
                ].join(" ")}
              >
                {step.label}
              </span>
            </div>
            {!isLast && (
              <span
                aria-hidden
                className={`mx-[14px] h-px w-12 ${
                  index < currentIndex ? "bg-primary" : "bg-border"
                }`}
              />
            )}
          </div>
        );
      })}
    </div>
  );
}
