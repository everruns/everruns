import { render, screen } from "@testing-library/react";
import { AgUiSetupGuidance } from "@/components/apps/ag-ui-setup-guidance";

describe("AgUiSetupGuidance", () => {
  it("renders the endpoint and anonymous status", () => {
    render(
      <AgUiSetupGuidance
        endpointUrl="https://example.com/api/v1/apps/app-123/ag-ui"
        isPublished={true}
        anonymousEnabled={true}
      />,
    );

    expect(screen.getByText("Anonymous")).toBeInTheDocument();
    expect(screen.getByText("Ready for AG-UI clients")).toBeInTheDocument();
    expect(screen.getByText("https://example.com/api/v1/apps/app-123/ag-ui")).toBeInTheDocument();
    expect(screen.getByText("Responses stream back as AG-UI SSE events.")).toBeInTheDocument();
  });
});
