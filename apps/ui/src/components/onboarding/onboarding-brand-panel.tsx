/**
 * Presentational navy brand panel for the onboarding shell.
 *
 * Renders the gold-on-navy chrome from the approved onboarding design: a gold
 * dot grid, the Borromean-rings motif bleeding off the top-right, an eyebrow,
 * a headline, a 3-item feature checklist, and a footer tagline.
 *
 * Pure/presentational — no data fetching, no routing.
 *
 * Design note on the dot grid: the global `.bg-brand-dots` utility paints navy
 * dots on a light surface (gold only in dark mode). This panel is always
 * gold-on-navy regardless of theme, so it carries the gold radial-gradient as a
 * scoped inline style rather than reusing `.bg-brand-dots`. The 22px cell and
 * 0.13 gold opacity mirror the design frames.
 */
import { Check } from "lucide-react";

export interface OnboardingFeature {
  label: string;
  done?: boolean;
}

export interface OnboardingBrandPanelProps {
  /** Gold mono-caps eyebrow, e.g. "Step 2 / 4" or "Setup complete". */
  eyebrow?: string;
  /** Large headline copy. */
  headline: string;
  /** Feature checklist; done items get a gold check, pending a muted dot. */
  features?: OnboardingFeature[];
  /** Footer tagline. */
  footer?: string;
}

const DEFAULT_FEATURES: OnboardingFeature[] = [
  { label: "Durable execution engine" },
  { label: "Agent harnesses & capabilities" },
  { label: "Session traces & evals" },
];

const DEFAULT_FOOTER = "Durable Agentic Harness · Unstoppable agents";

// Gold dot grid on navy. Kept inline because the panel is always gold-on-navy,
// unlike the theme-aware global `.bg-brand-dots` utility.
const dotGridStyle: React.CSSProperties = {
  backgroundImage: "radial-gradient(circle at center, hsl(43 60% 53% / 0.13) 1px, transparent 1px)",
  backgroundSize: "22px 22px",
};

export function OnboardingBrandPanel({
  eyebrow,
  headline,
  features = DEFAULT_FEATURES,
  footer = DEFAULT_FOOTER,
}: OnboardingBrandPanelProps) {
  return (
    <div className="relative hidden w-[440px] flex-shrink-0 flex-col justify-between overflow-hidden bg-primary px-9 py-10 text-primary-foreground lg:flex">
      {/* Gold dot grid */}
      <div aria-hidden className="pointer-events-none absolute inset-0" style={dotGridStyle} />
      {/* Borromean rings, bleeding off the top-right */}
      <svg
        aria-hidden
        width="460"
        height="460"
        viewBox="0 0 512 512"
        className="absolute -right-32 -top-[70px] text-accent opacity-[0.18]"
        fill="none"
        stroke="currentColor"
        strokeWidth="14"
      >
        <circle cx="256" cy="214" r="120" />
        <circle cx="186" cy="309" r="120" />
        <circle cx="326" cy="309" r="120" />
      </svg>

      {eyebrow ? (
        <div className="relative font-mono text-[11px] uppercase tracking-[0.18em] text-accent/90">
          {eyebrow}
        </div>
      ) : (
        <div className="relative" />
      )}

      <div className="relative">
        <p className="max-w-[320px] text-[26px] font-semibold leading-[1.28] tracking-[-0.02em]">
          {headline}
        </p>
      </div>

      <div className="relative">
        <ul className="mb-6 flex flex-col gap-[11px]">
          {features.map((feature) => (
            <li
              key={feature.label}
              className={`flex items-center gap-[11px] text-[13px] ${
                feature.done ? "text-primary-foreground/90" : "text-primary-foreground/55"
              }`}
            >
              {feature.done ? (
                <span className="flex h-[18px] w-[18px] flex-shrink-0 items-center justify-center border border-accent bg-accent/[0.18] text-accent">
                  <Check className="icon-sharp h-[11px] w-[11px]" strokeWidth={2.5} />
                </span>
              ) : (
                <span className="flex h-[18px] w-[18px] flex-shrink-0 items-center justify-center border border-primary-foreground/20 text-[10px] text-primary-foreground/35">
                  ·
                </span>
              )}
              {feature.label}
            </li>
          ))}
        </ul>
        <div className="font-mono text-[11px] tracking-[0.04em] text-primary-foreground/45">
          {footer}
        </div>
      </div>
    </div>
  );
}
