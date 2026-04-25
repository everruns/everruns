import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import OrganisationsPage from "@/app/(main)/settings/organisations/page";

const mockPush = jest.fn();
const mockSetCurrentOrg = jest.fn();
const mockMutateAsync = jest.fn();

jest.mock("next/navigation", () => ({
  useRouter: () => ({
    push: mockPush,
  }),
}));

jest.mock("@/providers/org-provider", () => ({
  useOrg: () => ({
    currentOrg: { public_id: "org-1", name: "Current Org", role: "owner" },
    organizations: [
      { public_id: "org-1", name: "Current Org", role: "owner" },
      { public_id: "org-2", name: "Second Org", role: "member" },
    ],
    setCurrentOrg: mockSetCurrentOrg,
  }),
}));

jest.mock("@/hooks/use-organizations", () => ({
  useCreateOrganization: () => ({
    mutateAsync: mockMutateAsync,
    isPending: false,
    isError: false,
    error: null,
  }),
}));

describe("OrganisationsPage", () => {
  beforeEach(() => {
    mockPush.mockClear();
    mockSetCurrentOrg.mockClear();
    mockMutateAsync.mockClear();
  });

  it("renders accessible organisations separately from general settings", () => {
    render(<OrganisationsPage />);

    expect(screen.getByRole("heading", { name: "Organisations" })).toBeInTheDocument();
    expect(screen.getByText("Current Org")).toBeInTheDocument();
    expect(screen.getByText("Second Org")).toBeInTheDocument();
    expect(screen.getByText("org-1")).toBeInTheDocument();
    expect(screen.getByText("org-2")).toBeInTheDocument();
    expect(screen.getByText("Current")).toBeInTheDocument();
  });

  it("switches to another organisation", () => {
    render(<OrganisationsPage />);

    fireEvent.click(screen.getByRole("button", { name: "Switch" }));

    expect(mockSetCurrentOrg).toHaveBeenCalledWith({
      public_id: "org-2",
      name: "Second Org",
      role: "member",
    });
  });

  it("creates a new organisation and routes to setup", async () => {
    mockMutateAsync.mockResolvedValue({ id: "org-3", name: "New Org" });

    render(<OrganisationsPage />);

    fireEvent.click(screen.getByRole("button", { name: "Create Organisation" }));
    fireEvent.change(screen.getByLabelText("Name"), { target: { value: "New Org" } });
    fireEvent.click(screen.getByRole("button", { name: "Create" }));

    await waitFor(() => {
      expect(mockMutateAsync).toHaveBeenCalledWith({ name: "New Org" });
      expect(mockSetCurrentOrg).toHaveBeenCalledWith({
        public_id: "org-3",
        name: "New Org",
        role: "owner",
      });
      expect(mockPush).toHaveBeenCalledWith("/orgs/org-3/setup");
    });
  });
});
