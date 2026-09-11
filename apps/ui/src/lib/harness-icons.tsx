// Harness glyphs: built-in harnesses declare an `icon` name in their backend
// definition (crates/server/src/harnesses/*.rs). Custom harnesses carry no icon
// and fall back to the neutral shield used across harness surfaces.
//
// The stored name is only ever a key into this map; unknown names fall back.
// Unlike `CapabilityIcon`, this renderer has no embedded-SVG branch, so a
// database-sourced icon can never put attacker-controlled markup on the page
// (TM-WEB-001).

import { forwardRef, type SVGProps } from "react";
import { BarChart3, Shield, SquareDashed } from "lucide-react";
import { capabilityIconMap, type IconComponent } from "@/lib/capability-icons";

/**
 * Everruns mark (monochrome): three rings on the vertices of an equilateral
 * triangle, centered on their centroid. Geometry mirrors `logo-mono.svg`.
 */
const EverrunsIcon = forwardRef<SVGSVGElement, SVGProps<SVGSVGElement>>((props, ref) => (
  <svg
    ref={ref}
    xmlns="http://www.w3.org/2000/svg"
    viewBox="0 0 512 512"
    fill="none"
    stroke="currentColor"
    // ~1.3px at a 16px render, matching the lucide siblings' stroke weight.
    strokeWidth={42}
    strokeLinecap="round"
    strokeLinejoin="round"
    {...props}
  >
    <circle cx="256" cy="183.64" r="120" />
    <circle cx="193.33" cy="292.18" r="120" />
    <circle cx="318.67" cy="292.18" r="120" />
  </svg>
));
EverrunsIcon.displayName = "EverrunsIcon";

/**
 * Icon names usable by harness definitions. Capability icon names are reused so
 * a harness can share a glyph with the capability it is built around.
 */
export const harnessIconMap: Record<string, IconComponent> = {
  ...capabilityIconMap,
  everruns: EverrunsIcon,
  "square-dashed": SquareDashed,
  "bar-chart": BarChart3,
};

/** Resolve a harness icon name, falling back to the neutral harness glyph. */
export function getHarnessIcon(iconName?: string | null): IconComponent {
  if (!iconName) return Shield;
  return harnessIconMap[iconName] ?? Shield;
}

interface HarnessIconProps extends SVGProps<SVGSVGElement> {
  icon?: string | null;
}

export function HarnessIcon({ icon, ...props }: HarnessIconProps) {
  const Icon = getHarnessIcon(icon);
  return <Icon {...props} />;
}
