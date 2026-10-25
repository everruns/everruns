import { createRef } from "react";
import { render } from "@testing-library/react";
import * as icons from "@/components/icons/facet-icons";
import { facetIconData, type FacetIconName } from "@/components/icons/facet-icon-data";
import { facetIconSvg } from "@/components/icons/facet-icon-data";

function parseSvg(markup: string) {
  return new DOMParser().parseFromString(markup, "image/svg+xml").documentElement;
}

describe("Facet domain identity", () => {
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
