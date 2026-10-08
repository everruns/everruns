import {
  createSandboxTemplateSpec,
  nextSandboxBindingName,
} from "@/components/agents/sandbox-policy-editor";
import type { SandboxTargetDescriptor } from "@/lib/api/types";

const daytona: SandboxTargetDescriptor = {
  kind: "managed",
  provider: "daytona",
  display_name: "Daytona",
  icon: "daytona",
  credential_sources: ["session_user", "agent", "organization"],
  available: true,
  capabilities: {
    native_processes: true,
    packages: true,
    pty: true,
    ports: true,
    portable_checkpoint: true,
    network_enforced: false,
  },
  containment_levels: ["isolated"],
  durability: "checkpointed",
};

const modal: SandboxTargetDescriptor = {
  ...daytona,
  provider: "modal",
  capabilities: { ...daytona.capabilities, portable_checkpoint: false },
  durability: "provider_snapshot",
};

describe("SandboxPolicyEditor helpers", () => {
  it("creates a Modal profile that recovers through provider snapshots", () => {
    expect(createSandboxTemplateSpec(modal)).toEqual({
      target: { kind: "managed", provider: "modal", credential: { source: "session_user" } },
      durability: "provider_snapshot",
      lifecycle: { idle_after_seconds: 180, idle_action: "checkpoint_and_stop" },
      bootstrap: { commands: [] },
    });
    expect(nextSandboxBindingName(modal, {})).toBe("modal");
  });

  it("creates a recoverable profile from the deployment descriptor", () => {
    expect(createSandboxTemplateSpec(daytona)).toEqual({
      target: { kind: "managed", provider: "daytona", credential: { source: "session_user" } },
      durability: "checkpointed",
      lifecycle: { idle_after_seconds: 180, idle_action: "checkpoint_and_stop" },
      bootstrap: { commands: [] },
    });
  });

  it("generates a unique addressable name", () => {
    expect(
      nextSandboxBindingName(daytona, {
        daytona: createSandboxTemplateSpec(daytona),
        "daytona-2": createSandboxTemplateSpec(daytona),
      }),
    ).toBe("daytona-3");
  });
});
