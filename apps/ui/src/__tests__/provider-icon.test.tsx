import { render } from "@testing-library/react";
import {
  ProviderIcon,
  getProviderLabel,
  getProviderDescription,
} from "@/components/providers/provider-icon";
import type { DriverId } from "@/lib/api/types";

describe("ProviderIcon", () => {
  it("uses the shared Facet provider fallback for an unknown provider type", () => {
    const { container } = render(<ProviderIcon providerType={"unknown" as DriverId} />);
    expect(container.querySelector("img")).not.toBeInTheDocument();
    expect(container.querySelector("svg")).toHaveAttribute("data-facet-icon", "provider");
  });
});

describe("getProviderLabel", () => {
  it("returns type string for unknown provider", () => {
    expect(getProviderLabel("unknown" as DriverId)).toBe("unknown");
  });
});

describe("getProviderDescription", () => {
  it("returns an empty string for an unknown provider", () => {
    expect(getProviderDescription("unknown" as DriverId)).toBe("");
  });
});
