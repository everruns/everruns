import { fireEvent, render, screen } from "@testing-library/react";
import { SandboxPolicyEditor } from "@/components/agents/sandbox-policy-editor";
import type { OrganizationConnection } from "@/lib/api/organization-connections";
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
    provider: "e2b",
    display_name: "E2B",
    icon: "cloud",
    credential_sources: ["session_user", "agent", "organization"],
    available: true,
    capabilities,
    containment_levels: ["isolated"],
    durability: "provider_snapshot",
  },
  {
    kind: "managed",
    provider: "daytona",
    display_name: "Daytona",
    icon: "daytona",
    credential_sources: ["session_user", "agent", "organization"],
    available: true,
    capabilities: { ...capabilities, portable_checkpoint: true },
    containment_levels: ["isolated"],
    durability: "checkpointed",
  },
  {
    kind: "managed",
    provider: "modal",
    display_name: "Modal",
    icon: "cloud",
    credential_sources: ["session_user", "agent", "organization"],
    available: true,
    capabilities,
    containment_levels: ["isolated"],
    durability: "provider_snapshot",
  },
  {
    kind: "host",
    display_name: "Host",
    icon: "server",
    credential_sources: [],
    available: false,
    reason: "host execution is not wired into the control plane",
    capabilities,
    containment_levels: [],
    durability: "none",
  },
];

let organizationConnections: OrganizationConnection[] = [];

jest.mock("@/hooks", () => ({
  useSandboxTargets: () => ({ data: { items: targets }, isLoading: false, error: null }),
  useSandboxTemplates: () => ({ data: [] }),
  useOrganizationConnections: () => ({ data: organizationConnections }),
}));

afterEach(() => {
  organizationConnections = [];
});

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

const daytonaPolicy = (
  credential?: SandboxTemplateSpec["target"]["credential"],
): SandboxPolicy => ({
  mode: "fixed",
  default: "daytona",
  templates: {
    daytona: {
      target: { kind: "managed", provider: "daytona", ...(credential ? { credential } : {}) },
      durability: "checkpointed",
      lifecycle: { idle_after_seconds: 180, idle_action: "checkpoint_and_stop" },
      bootstrap: { commands: [] },
    },
  },
});

const e2bPolicy = (): SandboxPolicy => ({
  mode: "fixed",
  default: "e2b",
  templates: {
    e2b: {
      target: {
        kind: "managed",
        provider: "e2b",
        options: { template: "base", timeout_seconds: 3600 },
      },
      durability: "provider_snapshot",
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

    expect(screen.getByLabelText("Provider account")).toBeInTheDocument();
    expect(screen.getByLabelText("Runtime")).toBeInTheDocument();
    expect(screen.getByLabelText("Image")).toBeInTheDocument();
    expect(screen.getByLabelText("CPU cores")).toHaveValue(2);
    expect(screen.getByLabelText("Workspace path")).toHaveAttribute("placeholder", "/workspace");
    expect(screen.queryByLabelText("Compute size")).not.toBeInTheDocument();
    expect(screen.queryByLabelText("Snapshot")).not.toBeInTheDocument();
  });

  it("shows E2B options instead of Daytona options", () => {
    render(<SandboxPolicyEditor value={e2bPolicy()} onChange={jest.fn()} />);

    expect(screen.getByLabelText("E2B template")).toHaveValue("base");
    expect(screen.getByLabelText("Provider timeout (seconds)")).toHaveValue(3600);
    expect(screen.getByLabelText("Workspace path")).toHaveAttribute("placeholder", "/home/user");
    expect(screen.queryByLabelText("Compute size")).not.toBeInTheDocument();
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

  it("offers Daytona the network policy but not E2B", () => {
    const onChange = jest.fn();
    const { unmount } = render(<SandboxPolicyEditor value={daytonaPolicy()} onChange={onChange} />);
    expect(screen.getByLabelText("Outbound network")).toHaveTextContent("Open");
    expect(screen.queryByLabelText("Allowed domains")).not.toBeInTheDocument();
    unmount();

    render(<SandboxPolicyEditor value={e2bPolicy()} onChange={jest.fn()} />);
    expect(screen.queryByLabelText("Outbound network")).not.toBeInTheDocument();
  });
});

describe("SandboxPolicyEditor for an Agent with triggers", () => {
  const warning = "Triggered runs have no user account; they use the organization account.";

  it("warns that triggered runs use the organization account", () => {
    render(<SandboxPolicyEditor value={daytonaPolicy()} onChange={jest.fn()} hasTriggers />);

    expect(screen.getByText(warning)).toBeInTheDocument();
  });

  it("stays quiet without triggers or with an explicit account", () => {
    const { unmount } = render(
      <SandboxPolicyEditor value={daytonaPolicy()} onChange={jest.fn()} />,
    );
    expect(screen.queryByText(warning)).not.toBeInTheDocument();
    unmount();

    render(
      <SandboxPolicyEditor
        value={daytonaPolicy({ source: "agent" })}
        onChange={jest.fn()}
        hasTriggers
      />,
    );
    expect(screen.queryByText(warning)).not.toBeInTheDocument();
  });
});

describe("SandboxPolicyEditor provider account picker", () => {
  const account = (id: string, name: string, provider: string): OrganizationConnection => ({
    id,
    name,
    provider,
    connected_at: "2026-10-01T00:00:00Z",
    updated_at: "2026-10-01T00:00:00Z",
  });

  it("labels the selected account with the provider and says who pays", () => {
    render(<SandboxPolicyEditor value={daytonaPolicy()} onChange={jest.fn()} />);

    expect(screen.getByLabelText("Provider account")).toHaveTextContent("Daytona · Personal");
    expect(
      screen.getByText(
        "Each person who starts a Session uses their own Daytona connection, and their account pays.",
      ),
    ).toBeInTheDocument();
  });

  it("describes Agent and organization accounts in plain words", () => {
    const { unmount } = render(
      <SandboxPolicyEditor value={daytonaPolicy({ source: "agent" })} onChange={jest.fn()} />,
    );
    expect(screen.getByLabelText("Provider account")).toHaveTextContent("Daytona · This Agent");
    expect(
      screen.getByText(
        "Uses the Daytona connection on this Agent's Virtual User; that account pays for every Session.",
      ),
    ).toBeInTheDocument();
    unmount();

    render(
      <SandboxPolicyEditor
        value={daytonaPolicy({ source: "organization" })}
        onChange={jest.fn()}
      />,
    );
    expect(screen.getByLabelText("Provider account")).toHaveTextContent(
      "Daytona · Organization account",
    );
    expect(
      screen.getByText("A shared organization Daytona account pays, whoever starts the Session."),
    ).toBeInTheDocument();
  });

  it("names organization accounts after their provider", () => {
    organizationConnections = [
      account("acc-1", "Reviews", "daytona"),
      account("acc-2", "Data team", "modal"),
    ];
    render(
      <SandboxPolicyEditor
        value={daytonaPolicy({ source: "organization", connection_id: "acc-1" })}
        onChange={jest.fn()}
      />,
    );

    expect(screen.getByLabelText("Organization account")).toHaveTextContent("Daytona · Reviews");
    expect(screen.queryByText(/has no Daytona account yet/)).not.toBeInTheDocument();
    expect(
      screen.getByRole("link", { name: "Manage organization provider accounts" }),
    ).toHaveAttribute("href", "/sandboxes/accounts");
  });

  it("links to Provider accounts when the organization has none for the provider", () => {
    organizationConnections = [account("acc-2", "Data team", "modal")];
    render(
      <SandboxPolicyEditor
        value={daytonaPolicy({ source: "organization" })}
        onChange={jest.fn()}
      />,
    );

    expect(screen.getByText(/This organization has no Daytona account yet/)).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Add a Daytona account" })).toHaveAttribute(
      "href",
      "/sandboxes/accounts",
    );
    expect(screen.queryByRole("combobox", { name: "Organization account" })).toBeNull();
  });
});
