import { fireEvent, render, screen } from "@testing-library/react";
import { SandboxPolicyEditor } from "@/components/agents/sandbox-policy-editor";
import type { SandboxPolicy, SandboxTargetDescriptor } from "@/lib/api/types";

const capabilities = {
  native_processes: true,
  packages: true,
  pty: true,
  ports: true,
  portable_checkpoint: false,
  network_enforced: false,
};

const targets: SandboxTargetDescriptor[] = [
  {
    kind: "managed",
    provider: "daytona",
    available: true,
    capabilities: { ...capabilities, portable_checkpoint: true },
    containment_levels: ["isolated"],
    durability: "checkpointed",
  },
  {
    kind: "managed",
    provider: "modal",
    available: true,
    capabilities,
    containment_levels: ["isolated"],
    durability: "provider_snapshot",
  },
];

jest.mock("@/hooks", () => ({
  useSandboxTargets: () => ({ data: { items: targets }, isLoading: false, error: null }),
  useSandboxTemplates: () => ({ data: [] }),
}));

const policy = (options: Record<string, unknown> = {}): SandboxPolicy => ({
  mode: "fixed",
  default: "modal",
  templates: {
    modal: {
      target: { kind: "managed", provider: "modal", options },
      durability: "provider_snapshot",
      lifecycle: { idle_after_seconds: 180, idle_action: "checkpoint_and_stop" },
      bootstrap: { commands: [] },
    },
  },
});

describe("SandboxPolicyEditor with a Modal template", () => {
  it("shows Modal's fields instead of Daytona's", () => {
    render(<SandboxPolicyEditor value={policy({ cpu: 2 })} onChange={jest.fn()} />);

    expect(screen.getByText(/Modal uses the connection/)).toBeInTheDocument();
    expect(screen.getByLabelText("Runtime")).toBeInTheDocument();
    expect(screen.getByLabelText("Image")).toBeInTheDocument();
    expect(screen.getByLabelText("CPU cores")).toHaveValue(2);
    expect(screen.getByLabelText("Workspace path")).toHaveAttribute("placeholder", "/workspace");
    expect(screen.queryByLabelText("Compute size")).not.toBeInTheDocument();
    expect(screen.queryByLabelText("Snapshot")).not.toBeInTheDocument();
  });

  it("stores numeric options as numbers and drops cleared ones", () => {
    const onChange = jest.fn();
    render(<SandboxPolicyEditor value={policy({ cpu: 2 })} onChange={onChange} />);

    fireEvent.change(screen.getByLabelText("Memory (MiB)"), { target: { value: "2048" } });
    expect(onChange.mock.lastCall[0].templates.modal.target.options).toEqual({
      cpu: 2,
      memory_mb: 2048,
    });

    fireEvent.change(screen.getByLabelText("CPU cores"), { target: { value: "" } });
    expect(onChange.mock.lastCall[0].templates.modal.target.options).toEqual({});
  });
});
