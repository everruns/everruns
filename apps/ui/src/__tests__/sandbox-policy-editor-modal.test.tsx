import { fireEvent, render, screen } from "@testing-library/react";
import { SandboxPolicyEditor } from "@/components/agents/sandbox-policy-editor";
import type { SandboxPolicy, SandboxTargetDescriptor, SandboxTemplateSpec } from "@/lib/api/types";

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
  {
    kind: "host",
    available: false,
    reason: "host execution is not wired into the control plane",
    capabilities,
    containment_levels: [],
    durability: "none",
  },
];

jest.mock("@/hooks", () => ({
  useSandboxTargets: () => ({ data: { items: targets }, isLoading: false, error: null }),
  useSandboxTemplates: () => ({ data: [] }),
}));

type Network = NonNullable<NonNullable<SandboxTemplateSpec["containment"]>["network"]>;

const policy = (options: Record<string, unknown> = {}, network?: Network): SandboxPolicy => ({
  mode: "fixed",
  default: "modal",
  templates: {
    modal: {
      target: { kind: "managed", provider: "modal", options },
      ...(network ? { containment: { level: "isolated", network } } : {}),
      durability: "provider_snapshot",
      lifecycle: { idle_after_seconds: 180, idle_action: "checkpoint_and_stop" },
      bootstrap: { commands: [] },
    },
  },
});

const daytonaPolicy = (): SandboxPolicy => ({
  mode: "fixed",
  default: "daytona",
  templates: {
    daytona: {
      target: { kind: "managed", provider: "daytona" },
      durability: "checkpointed",
      lifecycle: { idle_after_seconds: 180, idle_action: "checkpoint_and_stop" },
      bootstrap: { commands: [] },
    },
  },
});

describe("SandboxPolicyEditor with a Modal template", () => {
  it("explains recovery for the selected sandbox without catalog diagnostics", () => {
    render(<SandboxPolicyEditor value={daytonaPolicy()} onChange={jest.fn()} />);

    expect(
      screen.getByText(
        "Files are restored if this sandbox is replaced. Running processes restart.",
      ),
    ).toBeInTheDocument();
    expect(screen.queryByText("Filesystem plus replaceable compute")).not.toBeInTheDocument();
    expect(screen.queryByText(/host is unavailable/)).not.toBeInTheDocument();
  });

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

  it("injects the GitHub connection only with open egress", () => {
    const onChange = jest.fn();
    const { rerender } = render(<SandboxPolicyEditor value={policy()} onChange={onChange} />);

    fireEvent.click(screen.getByLabelText("Use my GitHub connection"));
    expect(onChange.mock.lastCall[0].templates.modal.target.options).toEqual({
      inject_connections: ["github"],
    });

    rerender(
      <SandboxPolicyEditor
        value={policy({ inject_connections: ["github"] }, { mode: "deny" })}
        onChange={onChange}
      />,
    );
    expect(screen.getByLabelText("Use my GitHub connection")).toBeDisabled();
    expect(screen.getByText(/Needs open network/)).toBeInTheDocument();
  });

  it("edits the domain allowlist as template containment", () => {
    const onChange = jest.fn();
    render(
      <SandboxPolicyEditor
        value={policy({}, { mode: "allowlist", allowed_hosts: ["pypi.org"] })}
        onChange={onChange}
      />,
    );

    fireEvent.change(screen.getByLabelText("Allowed domains"), {
      target: { value: "pypi.org\n *.pythonhosted.org \n" },
    });
    expect(onChange.mock.lastCall[0].templates.modal.containment).toEqual({
      level: "isolated",
      network: { mode: "allowlist", allowed_hosts: ["pypi.org", "*.pythonhosted.org"] },
    });
  });
});
