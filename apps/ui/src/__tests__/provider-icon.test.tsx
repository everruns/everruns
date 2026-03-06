import { render, screen } from "@testing-library/react";
import { ProviderIcon, getProviderLabel } from "@/components/providers/provider-icon";
import type { LlmProviderType } from "@/lib/api/types";

describe("ProviderIcon", () => {
  const providerTypes: LlmProviderType[] = ["openai", "openai_completions", "anthropic", "gemini"];

  it.each(providerTypes)("renders inline SVG for %s provider", (providerType) => {
    const { container } = render(<ProviderIcon providerType={providerType} />);
    const svg = container.querySelector("svg");
    expect(svg).toBeInTheDocument();
    // Verify it uses currentColor for theme-adaptive rendering
    const path = container.querySelector("path");
    expect(path).toBeInTheDocument();
  });

  it.each(providerTypes)("does not use <img> tags (no Next.js Image) for %s", (providerType) => {
    const { container } = render(<ProviderIcon providerType={providerType} />);
    const img = container.querySelector("img");
    expect(img).not.toBeInTheDocument();
  });

  it.each(providerTypes)("uses currentColor fill for %s", (providerType) => {
    const { container } = render(<ProviderIcon providerType={providerType} />);
    const pathsAndSvgs = container.querySelectorAll("[fill='currentColor']");
    expect(pathsAndSvgs.length).toBeGreaterThan(0);
  });

  it("renders fallback Server icon for unknown provider type", () => {
    // Cast to simulate an unknown provider type
    const { container } = render(<ProviderIcon providerType={"unknown" as LlmProviderType} />);
    // Should not render an SVG from our icon set
    const svg = container.querySelector("svg");
    // lucide-react Server icon renders as SVG too, so just confirm no <img>
    const img = container.querySelector("img");
    expect(img).not.toBeInTheDocument();
    expect(svg).toBeInTheDocument(); // Server icon from lucide
  });

  it("applies showBackground styles when true", () => {
    const { container } = render(<ProviderIcon providerType="openai" showBackground={true} />);
    const wrapper = container.firstElementChild;
    expect(wrapper?.className).toContain("bg-primary/10");
  });

  it("omits background styles when showBackground is false", () => {
    const { container } = render(<ProviderIcon providerType="openai" showBackground={false} />);
    const wrapper = container.firstElementChild;
    expect(wrapper?.className).not.toContain("bg-primary/10");
  });

  it("renders correct size for sm", () => {
    const { container } = render(<ProviderIcon providerType="openai" size="sm" />);
    const svg = container.querySelector("svg");
    expect(svg?.getAttribute("width")).toBe("16");
    expect(svg?.getAttribute("height")).toBe("16");
  });

  it("renders correct size for md (default)", () => {
    const { container } = render(<ProviderIcon providerType="anthropic" />);
    const svg = container.querySelector("svg");
    expect(svg?.getAttribute("width")).toBe("20");
    expect(svg?.getAttribute("height")).toBe("20");
  });

  it("renders correct size for lg", () => {
    const { container } = render(<ProviderIcon providerType="gemini" size="lg" />);
    const svg = container.querySelector("svg");
    expect(svg?.getAttribute("width")).toBe("24");
    expect(svg?.getAttribute("height")).toBe("24");
  });

  it("sets title attribute with provider label", () => {
    render(<ProviderIcon providerType="openai" />);
    expect(screen.getByTitle("OpenAI (Responses)")).toBeInTheDocument();
  });
});

describe("getProviderLabel", () => {
  it("returns correct label for openai", () => {
    expect(getProviderLabel("openai")).toBe("OpenAI (Responses)");
  });

  it("returns correct label for anthropic", () => {
    expect(getProviderLabel("anthropic")).toBe("Anthropic");
  });

  it("returns correct label for gemini", () => {
    expect(getProviderLabel("gemini")).toBe("Google Gemini");
  });

  it("returns type string for unknown provider", () => {
    expect(getProviderLabel("unknown" as LlmProviderType)).toBe("unknown");
  });
});
