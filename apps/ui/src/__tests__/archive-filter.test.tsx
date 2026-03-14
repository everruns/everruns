import { render, screen, fireEvent } from "@testing-library/react";
import { ArchiveFilter } from "@/components/archive-filter";

describe("ArchiveFilter", () => {
  it("renders a Filter button", () => {
    render(<ArchiveFilter showArchived={false} onShowArchivedChange={jest.fn()} />);

    expect(screen.getByRole("button", { name: /filter/i })).toBeInTheDocument();
  });

  it("does not show badge when filter is inactive", () => {
    render(<ArchiveFilter showArchived={false} onShowArchivedChange={jest.fn()} />);

    expect(screen.queryByText("1")).not.toBeInTheDocument();
  });

  it("shows badge with count when filter is active", () => {
    render(<ArchiveFilter showArchived={true} onShowArchivedChange={jest.fn()} />);

    expect(screen.getByText("1")).toBeInTheDocument();
  });

  it("uses the default label when none provided", () => {
    render(<ArchiveFilter showArchived={false} onShowArchivedChange={jest.fn()} />);

    fireEvent.click(screen.getByRole("button", { name: /filter/i }));

    expect(screen.getByText("Show archived")).toBeInTheDocument();
  });

  it("uses a custom label when provided", () => {
    render(
      <ArchiveFilter
        showArchived={false}
        onShowArchivedChange={jest.fn()}
        label="Show archived agents"
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: /filter/i }));

    expect(screen.getByText("Show archived agents")).toBeInTheDocument();
  });

  it("calls onShowArchivedChange when checkbox item is clicked", () => {
    const onChange = jest.fn();
    render(<ArchiveFilter showArchived={false} onShowArchivedChange={onChange} />);

    // Open dropdown
    fireEvent.click(screen.getByRole("button", { name: /filter/i }));

    // Click the checkbox item
    fireEvent.click(screen.getByText("Show archived"));

    expect(onChange).toHaveBeenCalled();
    expect(onChange.mock.calls[0][0]).toBe(true);
  });

  it("shows Filters label in dropdown header", () => {
    render(<ArchiveFilter showArchived={false} onShowArchivedChange={jest.fn()} />);

    fireEvent.click(screen.getByRole("button", { name: /filter/i }));

    expect(screen.getByText("Filters")).toBeInTheDocument();
  });
});
