import { fireEvent, render, screen } from "@testing-library/react";
import { VirtualUserSelect } from "@/components/virtual-user/virtual-user-select";
import { useVirtualUser, useVirtualUsers } from "@/hooks/use-virtual-users";

jest.mock("@/hooks/use-virtual-users");
const list = jest.mocked(useVirtualUsers);
const detail = jest.mocked(useVirtualUser);
const alice = { id: "user-a", name: "Alice", usage: "end_user", status: "active" };
const archived = { id: "user-b", name: "Archived", usage: "end_user", status: "archived" };
const service = { id: "user-service", name: "Service", usage: "service", status: "active" };
const next = jest.fn();
beforeEach(() => {
  jest.resetAllMocks();
  list.mockReturnValue({
    data: [alice, archived, service],
    hasNextPage: true,
    fetchNextPage: next,
    isFetchingNextPage: false,
    isLoading: false,
  } as unknown as ReturnType<typeof useVirtualUsers>);
  detail.mockReturnValue({ data: alice } as unknown as ReturnType<typeof useVirtualUser>);
});
it("passes search to the paginated API without losing the selected user or usage restrictions", async () => {
  const change = jest.fn();
  const view = render(
    <VirtualUserSelect
      value="user-a"
      onValueChange={change}
      usage="end_user"
      includeNone={false}
    />,
  );
  fireEvent.click(screen.getByRole("combobox", { name: "Select virtual user" }));
  const input = await screen.findByRole("combobox", { name: "Search virtual users" });
  expect(screen.queryByRole("option", { name: "Service" })).not.toBeInTheDocument();
  expect(screen.getByRole("option", { name: "Archived (archived)" })).toHaveAttribute(
    "aria-disabled",
    "true",
  );
  fireEvent.click(screen.getByRole("button", { name: "Load more virtual users" }));
  expect(next).toHaveBeenCalled();
  fireEvent.change(input, { target: { value: "  Carol  " } });
  expect(list).toHaveBeenLastCalledWith({
    includeArchived: true,
    usage: "end_user",
    search: "Carol",
  });
  list.mockReturnValue({ data: [], hasNextPage: false, isLoading: false } as unknown as ReturnType<
    typeof useVirtualUsers
  >);
  view.rerender(
    <VirtualUserSelect
      value="user-a"
      onValueChange={change}
      usage="end_user"
      includeNone={false}
    />,
  );
  expect(screen.getByRole("combobox", { name: "Select virtual user" })).toHaveTextContent("Alice");
  expect(screen.getByRole("status")).toHaveTextContent("No results found");
  expect(change).not.toHaveBeenCalled();
});
it("preserves an archived selected user outside the current page as a disabled entry", async () => {
  detail.mockReturnValue({ data: archived } as unknown as ReturnType<typeof useVirtualUser>);
  list.mockReturnValue({ data: [], isLoading: false } as unknown as ReturnType<
    typeof useVirtualUsers
  >);
  render(<VirtualUserSelect value="user-b" onValueChange={jest.fn()} usage="end_user" />);
  fireEvent.click(screen.getByRole("combobox", { name: "Select virtual user" }));
  expect(await screen.findByRole("option", { name: "Archived" })).toHaveAttribute(
    "aria-disabled",
    "true",
  );
});
