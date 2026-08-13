import { render, screen } from "@testing-library/react";

import { ResourceNotFound } from "@/components/resource-not-found";

describe("ResourceNotFound", () => {
  it("renders resource-specific copy, actions, and id", () => {
    render(
      <ResourceNotFound
        title="Harness not found"
        description="This harness may have been deleted, moved to another organization, or the URL may be wrong."
        backHref="/harnesses"
        backLabel="Back to harnesses"
        resourceId="harness_019defa39e937b0c999ef870e8ded53b23"
      />,
    );

    expect(screen.getByText("404 / Resource not found")).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "Harness not found" })).toBeInTheDocument();
    expect(screen.getByRole("link", { name: /Back to harnesses/ })).toHaveAttribute(
      "href",
      "/harnesses",
    );
    expect(screen.getByRole("link", { name: /Go to chats/ })).toHaveAttribute("href", "/chats");
    expect(screen.getByText("harness_019defa39e937b0c999ef870e8ded53b23")).toBeInTheDocument();
  });
});
