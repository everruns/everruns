import { act, fireEvent, render, screen } from "@testing-library/react";
import NewAgentPage from "@/app/(main)/agents/new/page";
import NewHarnessPage from "@/app/(main)/harnesses/new/page";

const push = jest.fn();
const back = jest.fn();
const createAgent = jest.fn();
const createHarness = jest.fn();

jest.mock("next/navigation", () => ({
  usePathname: () => "/agents/new",
  useRouter: () => ({ push, back }),
}));

jest.mock("next/link", () => ({
  __esModule: true,
  default: ({ children, href, ...props }: React.ComponentPropsWithoutRef<"a">) => (
    <a href={href} {...props}>
      {children}
    </a>
  ),
}));

jest.mock("@/hooks", () => ({
  useCreateAgent: () => ({ mutateAsync: createAgent, isPending: false, error: null }),
  useCreateHarness: () => ({ mutateAsync: createHarness, isPending: false, error: null }),
  useCapabilities: () => ({ data: [] }),
  useAgentNameAvailability: () => ({ isChecking: false, available: true }),
  useHarnessNameAvailability: () => ({ isChecking: false, available: true }),
  usePageTitle: () => undefined,
}));

jest.mock("@/components/ui/prompt-editor", () => ({
  PromptEditor: ({
    id,
    value,
    onChange,
  }: {
    id: string;
    value: string;
    onChange: (value: string) => void;
  }) => <textarea id={id} value={value} onChange={(event) => onChange(event.target.value)} />,
}));

jest.mock("@/components/models/model-picker", () => ({
  ModelPicker: () => <div data-testid="model-picker" />,
}));

jest.mock("@/components/harness/harness-select", () => ({
  HarnessSelect: ({
    id,
    value,
    onValueChange,
  }: {
    id?: string;
    value: string;
    onValueChange: (value: string) => void;
  }) => (
    <select id={id} value={value} onChange={(event) => onValueChange(event.target.value)}>
      <option value="">None</option>
      <option value="harness_123">Generic</option>
    </select>
  ),
}));

jest.mock("@/components/agents/capability-selector", () => ({
  CapabilitySelector: () => <div data-testid="capability-selector" />,
}));

jest.mock("@/components/initial-files-editor", () => ({
  InitialFilesEditor: () => <div data-testid="initial-files-editor" />,
}));

describe("create editor layouts", () => {
  beforeEach(() => {
    jest.clearAllMocks();
    createAgent.mockResolvedValue({ id: "agent_123" });
    createHarness.mockResolvedValue({ id: "harness_456" });
  });

  it("structures New Agent into navigable sections and submits network access", async () => {
    render(<NewAgentPage />);

    expect(screen.getByRole("navigation", { name: "Jump to section" })).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Identity" })).toHaveAttribute("href", "#identity");
    expect(screen.getByRole("link", { name: "Network" })).toHaveAttribute("href", "#network");

    fireEvent.change(screen.getByLabelText("Display Name"), {
      target: { value: "Support Agent" },
    });
    fireEvent.change(screen.getByLabelText(/Harness/), { target: { value: "harness_123" } });
    fireEvent.change(screen.getByLabelText("System Prompt"), {
      target: { value: "You are helpful." },
    });
    fireEvent.change(screen.getByLabelText("Allowed hosts"), {
      target: { value: "api.example.com" },
    });

    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Create Agent" }));
    });

    expect(createAgent).toHaveBeenCalledWith(
      expect.objectContaining({
        name: "support-agent",
        harness_id: "harness_123",
        network_access: { allowed: ["api.example.com"] },
      }),
    );
    expect(push).toHaveBeenCalledWith("/agents/agent_123");
  });

  it("uses the same section structure for New Harness", async () => {
    render(<NewHarnessPage />);

    expect(screen.getByRole("navigation", { name: "Jump to section" })).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Behavior" })).toHaveAttribute("href", "#behavior");
    expect(screen.getByRole("link", { name: "Files" })).toHaveAttribute("href", "#files");

    fireEvent.change(screen.getByLabelText("Display Name"), {
      target: { value: "Safe Harness" },
    });
    fireEvent.change(screen.getByLabelText("Blocked hosts"), {
      target: { value: "internal.example.com" },
    });

    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Create Harness" }));
    });

    expect(createHarness).toHaveBeenCalledWith(
      expect.objectContaining({
        name: "safe-harness",
        network_access: { blocked: ["internal.example.com"] },
      }),
    );
    expect(push).toHaveBeenCalledWith("/harnesses/harness_456");
  });
});
