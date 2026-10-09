import { createRef } from "react";
import { render } from "@testing-library/react";
import * as icons from "@/components/icons/facet-icons";
import { facetIconData, type FacetIconName } from "@/components/icons/facet-icon-data";
import { facetIconSvg } from "@/components/icons/facet-icon-data";
import { defaultNavigationSections } from "@/lib/navigation";
import { settingsNavigationSections } from "@/lib/settings-navigation";
import { registryDomainIcons } from "@/lib/registry-navigation";
import { AgentAvatar } from "@/components/agents/agent-avatar";
import { capabilityIconMap } from "@/lib/capability-icons";

function parseSvg(markup: string) {
  return new DOMParser().parseFromString(markup, "image/svg+xml").documentElement;
}

describe("Facet domain identity", () => {
  it("uses the same domain glyph in navigation and avatar fallbacks while retaining official logos", () => {
    const items = defaultNavigationSections.flatMap((section) => section.items);
    expect(items.find((item) => item.href === "/agents")?.icon).toBe(icons.AgentIcon);
    expect(items.find((item) => item.href === "/models")?.icon).toBe(registryDomainIcons.models);
    expect(registryDomainIcons.models).toBe(icons.ModelsIcon);
    expect(registryDomainIcons.mcpServers).toBe(capabilityIconMap.mcp);
    const settings = settingsNavigationSections.flatMap((section) => section.items);
    expect(settings.find((item) => item.href === "/settings/agent-experience")?.icon).toBe(
      icons.AgentExperienceIcon,
    );
    expect(settings.find((item) => item.href === "/settings/profile")?.icon).toBe(
      icons.AccountIcon,
    );
    const { container } = render(<AgentAvatar size={16} />);
    expect(container.querySelector("svg")).toHaveAttribute("data-facet-icon", "agent");
  });

  it("preserves the approved original Intent silhouette", () => {
    const { container } = render(<icons.AgentIcon />);
    const svg = container.querySelector("svg")!;
    expect(svg).toHaveAttribute("fill", "currentColor");
    expect(svg).toHaveAttribute("stroke", "none");
    expect([...svg.querySelectorAll("path")].map((path) => path.getAttribute("d"))).toEqual([
      "M4 7 11 3V10L4 14Z M4 16 19 8V15L4 23Z",
      "M14 2 19 5V6L14 9Z",
    ]);
  });

  it("forwards ref, size, color, class, and accessible overrides", () => {
    const ref = createRef<SVGSVGElement>();
    const { container } = render(
      <icons.SkillsIcon
        ref={ref}
        size={16}
        color="red"
        className="test-icon"
        aria-hidden={false}
        role="img"
        aria-label="Skills"
      />,
    );
    expect(ref.current).toBe(container.querySelector("svg"));
    expect(ref.current).toHaveAttribute("width", "16");
    expect(ref.current).toHaveAttribute("stroke", "currentColor");
    expect(ref.current).toHaveAttribute("color", "red");
    expect(ref.current).toHaveClass("test-icon");
    expect(ref.current).toHaveAttribute("aria-hidden", "false");
    expect(ref.current).toHaveAttribute("aria-label", "Skills");
  });

  it("exports every React master with identical geometry and paint, without active SVG content", () => {
    const covered = new Set<string>();
    for (const Icon of Object.values(icons)) {
      const { container, unmount } = render(<Icon />);
      const rendered = container.querySelector("svg")!;
      const name = rendered.getAttribute("data-facet-icon") as FacetIconName;
      covered.add(name);
      const exported = parseSvg(facetIconSvg(name));
      for (const attribute of [
        "viewBox",
        "fill",
        "stroke",
        "stroke-width",
        "stroke-linecap",
        "stroke-linejoin",
      ]) {
        expect(exported.getAttribute(attribute)).toBe(rendered.getAttribute(attribute));
      }
      expect([...exported.querySelectorAll("path")].map((path) => path.getAttribute("d"))).toEqual(
        [...rendered.querySelectorAll("path")].map((path) => path.getAttribute("d")),
      );
      expect(
        [...exported.children].every(
          (element) => element.tagName === "path" && element.attributes.length === 1,
        ),
      ).toBe(true);
      unmount();
    }
    expect([...covered].sort()).toEqual(Object.keys(facetIconData).sort());
  });
});
