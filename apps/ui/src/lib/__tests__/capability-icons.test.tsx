import { render } from "@testing-library/react";
import { CapabilityIcon } from "@/lib/capability-icons";

describe("CapabilityIcon", () => {
  it("renders a validated embedded plugin SVG", () => {
    const data = `data:image/svg+xml;base64,${btoa('<svg viewBox="0 0 1 1"></svg>')}`;
    const { container } = render(<CapabilityIcon icon={data} className="plugin-icon" />);

    const image = container.querySelector("img");
    expect(image).toHaveAttribute("src", data);
    expect(image).toHaveClass("plugin-icon");
  });

  it("does not load remote icon metadata", () => {
    const { container } = render(<CapabilityIcon icon="https://tracker.example/icon.svg" />);

    expect(container.querySelector("img")).not.toBeInTheDocument();
    expect(container.querySelector("svg")).toBeInTheDocument();
  });

  it("uses a neutral fallback for missing icon metadata", () => {
    const { container } = render(<CapabilityIcon />);

    expect(container.querySelector("svg")).toBeInTheDocument();
  });
});
