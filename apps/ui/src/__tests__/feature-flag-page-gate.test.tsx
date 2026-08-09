import { render, screen } from "@testing-library/react";
import { FeatureFlagPageGate } from "@/components/feature-flag-page-gate";

const mockUseFeatureFlag = jest.fn();

jest.mock("@/providers/feature-flags-provider", () => ({
  useFeatureFlag: (flag: string) => mockUseFeatureFlag(flag),
}));

jest.mock("next/link", () => ({
  __esModule: true,
  default: ({ children, ...props }: React.AnchorHTMLAttributes<HTMLAnchorElement>) => (
    <a {...props}>{children}</a>
  ),
}));

describe("FeatureFlagPageGate", () => {
  beforeEach(() => {
    mockUseFeatureFlag.mockReset();
  });

  it("hides route content and links to feature settings when disabled", () => {
    mockUseFeatureFlag.mockReturnValue(false);

    render(
      <FeatureFlagPageGate flag="skills" title="Skills">
        <div>Skills content</div>
      </FeatureFlagPageGate>,
    );

    expect(mockUseFeatureFlag).toHaveBeenCalledWith("skills");
    expect(screen.queryByText("Skills content")).not.toBeInTheDocument();
    expect(screen.getByText("Skills is not enabled")).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Settings → Features" })).toHaveAttribute(
      "href",
      "/settings/features",
    );
  });

  it("renders route content when enabled", () => {
    mockUseFeatureFlag.mockReturnValue(true);

    render(
      <FeatureFlagPageGate flag="skills" title="Skills">
        <div>Skills content</div>
      </FeatureFlagPageGate>,
    );

    expect(screen.getByText("Skills content")).toBeInTheDocument();
    expect(screen.queryByText("Skills is not enabled")).not.toBeInTheDocument();
  });
});
