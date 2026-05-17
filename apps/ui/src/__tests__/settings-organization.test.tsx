import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import OrganizationPage from "@/app/(main)/settings/organization/page";

const mockPush = jest.fn();
const mockSetCurrentOrg = jest.fn();
const mockMutateAsync = jest.fn();
const mockUpdateOrganization = jest.fn();
let mockOrganization = {
  id: "org-1",
  name: "Current Org",
  default_model_id: "model-1" as string | null,
  default_harness_id: "harness-generic",
  base_harness_id: "harness-base",
};

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
  useOrganization: () => ({
    data: mockOrganization,
    isLoading: false,
    error: null,
  }),
  useUpdateOrganization: () => ({
    mutateAsync: mockUpdateOrganization,
    isPending: false,
    isError: false,
    isSuccess: false,
    error: null,
  }),
  useCreateOrganization: () => ({
    mutateAsync: mockMutateAsync,
    isPending: false,
    isError: false,
    error: null,
  }),
}));

jest.mock("@/hooks", () => ({
  useHarnesses: () => ({
    data: [
      { id: "harness-generic", name: "Generic" },
      { id: "harness-base", name: "Base" },
    ],
  }),
  usePageTitle: jest.fn(),
}));

jest.mock("@/components/models/model-picker", () => ({
  ModelPicker: ({ value, onChange }: { value: string; onChange: (value: string) => void }) => (
    <select aria-label="Default Model" value={value} onChange={(e) => onChange(e.target.value)}>
      <option value="">No default model</option>
      <option value="model-1">Default Model</option>
    </select>
  ),
}));

jest.mock("@/components/harness/harness-select", () => ({
  HarnessSelect: ({
    value,
    onValueChange,
    placeholder,
  }: {
    value: string;
    onValueChange: (value: string) => void;
    placeholder: string;
  }) => (
    <select aria-label={placeholder} value={value} onChange={(e) => onValueChange(e.target.value)}>
      <option value="harness-generic">Generic</option>
      <option value="harness-base">Base</option>
    </select>
  ),
}));

describe("OrganizationPage", () => {
  beforeEach(() => {
    mockPush.mockClear();
    mockSetCurrentOrg.mockClear();
    mockMutateAsync.mockClear();
    mockUpdateOrganization.mockClear();
    mockUpdateOrganization.mockResolvedValue({});
    mockOrganization = {
      id: "org-1",
      name: "Current Org",
      default_model_id: "model-1",
      default_harness_id: "harness-generic",
      base_harness_id: "harness-base",
    };
  });

  it("renders accessible organizations separately from general settings", () => {
    render(<OrganizationPage />);

    expect(screen.getByRole("heading", { name: "Organization" })).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "Models" })).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "Harnesses" })).toBeInTheDocument();
    expect(screen.getByLabelText("Default Model")).toBeInTheDocument();
    expect(screen.getByLabelText("Select default harness")).toBeInTheDocument();
    expect(screen.getByLabelText("Select base harness")).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "Organizations" })).toBeInTheDocument();
    expect(screen.getByRole("table", { name: "Organizations" })).toBeInTheDocument();
    expect(screen.getByRole("columnheader", { name: "Organization" })).toBeInTheDocument();
    expect(screen.getByRole("columnheader", { name: "ID" })).toBeInTheDocument();
    expect(screen.getByRole("columnheader", { name: "Role" })).toBeInTheDocument();
    expect(screen.getByRole("columnheader", { name: "Status" })).toBeInTheDocument();
    expect(screen.getByRole("columnheader", { name: "Actions" })).toBeInTheDocument();
    expect(screen.getAllByText("Current Org").length).toBeGreaterThan(0);
    expect(screen.getByText("Second Org")).toBeInTheDocument();
    expect(screen.getByText("org-1")).toBeInTheDocument();
    expect(screen.getByText("org-2")).toBeInTheDocument();
    expect(screen.getByText("Current")).toBeInTheDocument();
  });

  it("switches to another organization", () => {
    render(<OrganizationPage />);

    fireEvent.click(screen.getByRole("button", { name: "Switch to Second Org" }));

    expect(mockSetCurrentOrg).toHaveBeenCalledWith({
      public_id: "org-2",
      name: "Second Org",
      role: "member",
    });
  });

  it("creates a new organization and routes to setup", async () => {
    mockMutateAsync.mockResolvedValue({ id: "org-3", name: "New Org" });

    render(<OrganizationPage />);

    fireEvent.click(screen.getByRole("button", { name: "Create Organization" }));
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

  it("autosaves organization identity changes", async () => {
    jest.useFakeTimers();

    render(<OrganizationPage />);

    fireEvent.change(screen.getByLabelText("Organization Name"), {
      target: { value: "Renamed Org" },
    });

    expect(screen.getByText("Unsaved changes")).toBeInTheDocument();

    await act(async () => {
      jest.advanceTimersByTime(700);
      await Promise.resolve();
    });

    await waitFor(() => {
      expect(mockUpdateOrganization).toHaveBeenCalledWith({
        name: "Renamed Org",
        default_model_id: "model-1",
        default_harness_id: "harness-generic",
        base_harness_id: "harness-base",
      });
    });

    jest.useRealTimers();
  });

  it("omits an empty default model from autosave payloads", async () => {
    jest.useFakeTimers();
    mockOrganization = {
      ...mockOrganization,
      default_model_id: null,
    };

    render(<OrganizationPage />);

    fireEvent.change(screen.getByLabelText("Organization Name"), {
      target: { value: "Renamed Org" },
    });

    await act(async () => {
      jest.advanceTimersByTime(700);
      await Promise.resolve();
    });

    await waitFor(() => {
      expect(mockUpdateOrganization).toHaveBeenCalledWith({
        name: "Renamed Org",
        default_harness_id: "harness-generic",
        base_harness_id: "harness-base",
      });
    });

    jest.useRealTimers();
  });

  it("ignores stale autosaves after the loaded organization changes", async () => {
    jest.useFakeTimers();
    let resolveSave: () => void = () => {};
    mockUpdateOrganization.mockReturnValueOnce(
      new Promise<void>((resolve) => {
        resolveSave = resolve;
      }),
    );

    const { rerender } = render(<OrganizationPage />);

    fireEvent.change(screen.getByLabelText("Organization Name"), {
      target: { value: "Renamed Org" },
    });

    await act(async () => {
      jest.advanceTimersByTime(700);
      await Promise.resolve();
    });

    mockOrganization = {
      id: "org-2",
      name: "Second Org",
      default_model_id: "model-1",
      default_harness_id: "harness-generic",
      base_harness_id: "harness-base",
    };

    rerender(<OrganizationPage />);

    await act(async () => {
      resolveSave();
      await Promise.resolve();
    });

    expect(screen.getByLabelText("Organization Name")).toHaveValue("Second Org");

    jest.useRealTimers();
  });
});
