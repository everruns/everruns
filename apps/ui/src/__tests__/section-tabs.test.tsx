import { fireEvent, render, screen } from "@testing-library/react";
import { SectionTabs } from "@/components/layout/page-layout";

describe("SectionTabs", () => {
  it("keeps route sections as links with a current-page marker", () => {
    render(
      <SectionTabs
        value="cost"
        aria-label="Session views"
        items={[
          { value: "transcript", label: "Transcript", href: "/sessions/123/transcript" },
          { value: "events", label: "Events", count: "12.4K", href: "/sessions/123/events" },
          { value: "cost", label: "Cost", href: "/sessions/123/cost" },
        ]}
      />,
    );

    expect(screen.getByRole("navigation", { name: "Session views" })).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Cost" })).toHaveAttribute("aria-current", "page");
    expect(screen.getByRole("link", { name: "Transcript" })).not.toHaveAttribute("aria-current");
    expect(screen.getByRole("link", { name: "Events" })).toHaveAttribute(
      "href",
      "/sessions/123/events",
    );
    expect(screen.getByText("12.4K")).toBeVisible();
  });

  it("changes local sections and respects disabled tabs", () => {
    const onValueChange = jest.fn();
    render(
      <SectionTabs
        value="edit"
        onValueChange={onValueChange}
        items={[
          { value: "edit", label: "Edit" },
          { value: "preview", label: "Preview" },
          { value: "disabled", label: "Disabled", disabled: true },
        ]}
      />,
    );

    expect(screen.getByRole("tab", { name: "Edit" })).toHaveAttribute("aria-selected", "true");
    fireEvent.click(screen.getByRole("tab", { name: "Preview" }));
    expect(onValueChange).toHaveBeenCalledWith("preview");
    onValueChange.mockClear();
    fireEvent.click(screen.getByRole("tab", { name: "Disabled" }));
    expect(onValueChange).not.toHaveBeenCalled();
  });
});
