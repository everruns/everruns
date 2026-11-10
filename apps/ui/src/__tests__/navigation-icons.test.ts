import { Activity } from "lucide-react";
import { HealthIcon } from "@/components/icons/facet-icons";
import { defaultNavigationSections } from "@/lib/navigation";
import { settingsNavigationSections } from "@/lib/settings-navigation";

// Two entries with the same icon in one sidebar section read as the same page
// at a glance. Lucide's Activity draws the same pulse line as Facet's Health
// icon, so it counts as Health too (Tracing once used it beside Health).
const sameDrawing = new Map<unknown, unknown>([[Activity, HealthIcon]]);

describe("sidebar icons", () => {
  it.each([
    ...settingsNavigationSections.map((section) => ["Settings", section] as const),
    ...defaultNavigationSections.map((section) => ["Main", section] as const),
  ])("%s / %s.label has no repeated icon", (_sidebar, section) => {
    const seen = new Map<unknown, string>();
    for (const item of section.items) {
      const drawing = sameDrawing.get(item.icon) ?? item.icon;
      const other = seen.get(drawing);
      expect(other ? `${item.name} repeats ${other}'s icon` : undefined).toBeUndefined();
      seen.set(drawing, item.name);
    }
  });
});
