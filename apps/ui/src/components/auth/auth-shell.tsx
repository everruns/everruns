/**
 * Presentational two-panel auth shell — the shared chrome for the login and
 * register pages (Frame 2 of the approved onboarding design).
 *
 * Mirrors `OnboardingShell` but without a stepper: a header bar (logo +
 * Everruns wordmark on the left, an optional right-side note such as
 * "Secured by …"), then a two-column body — the left pane holds the auth
 * `children` (form), the right pane is the shared navy `OnboardingBrandPanel`.
 * On small screens the brand panel hides and the layout collapses to a single
 * centered column, so the form must stand on its own on mobile.
 *
 * The shell is the inner framed surface (white `card`, 1px border, no shadow,
 * sharp corners), matching the design frames. The route layout
 * (`(auth)/layout.tsx`) provides the textured `bg-background bg-brand-dots`
 * page background.
 *
 * Pure/presentational — no data fetching, no routing.
 */
import type { ReactNode } from "react";
import Image from "next/image";
import {
  OnboardingBrandPanel,
  type OnboardingBrandPanelProps,
} from "@/components/onboarding/onboarding-brand-panel";

// Frame 2 default brand copy for the login page. Register overrides the
// headline with an on-brand variant.
const DEFAULT_HEADLINE =
  "Every step persisted — agents that survive restarts, deploys, and outages.";

export interface AuthShellProps {
  /** Optional right-aligned header note, e.g. "Secured by …". */
  headerNote?: ReactNode;
  /** Brand panel content for the right column. Headline defaults to Frame 2. */
  brand?: OnboardingBrandPanelProps;
  /** Left content pane (the auth form). */
  children: ReactNode;
}

export function AuthShell({ headerNote, brand, children }: AuthShellProps) {
  const brandProps: OnboardingBrandPanelProps = {
    eyebrow: "Everruns",
    headline: DEFAULT_HEADLINE,
    ...brand,
  };

  return (
    <div className="flex w-full max-w-[1200px] flex-col overflow-hidden border bg-card text-card-foreground">
      {/* Header bar */}
      <div className="flex h-[60px] flex-shrink-0 items-center justify-between border-b px-6 sm:px-10">
        <div className="flex items-center gap-[10px]">
          <Image src="/logo.svg" alt="Everruns" width={26} height={26} />
          <span className="text-[15px] font-semibold tracking-[-0.02em]">Everruns</span>
        </div>
        {headerNote && (
          <div className="font-mono text-[11px] text-muted-foreground">{headerNote}</div>
        )}
      </div>

      {/* Body */}
      <div className="flex flex-1 flex-col lg:flex-row">
        <div className="flex flex-1 flex-col justify-center px-6 py-10 sm:px-12 lg:px-16">
          <div className="mx-auto w-full max-w-[400px]">{children}</div>
        </div>
        <OnboardingBrandPanel {...brandProps} />
      </div>
    </div>
  );
}
