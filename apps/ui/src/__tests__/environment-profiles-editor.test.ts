import {
  createEnvironmentProfile,
  nextEnvironmentName,
} from "@/components/agents/environment-profiles-editor";
import type { EnvironmentTargetDescriptor } from "@/lib/api/types";

const daytona: EnvironmentTargetDescriptor = {
  kind: "managed",
  provider: "daytona",
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

describe("EnvironmentProfilesEditor helpers", () => {
  it("creates a recoverable profile from the deployment descriptor", () => {
    expect(createEnvironmentProfile(daytona)).toEqual({
      target: { kind: "managed", provider: "daytona" },
      durability: "checkpointed",
      lifecycle: { idle_after_seconds: 180, idle_action: "checkpoint_and_stop" },
      bootstrap: { commands: [] },
    });
  });

  it("generates a unique addressable name", () => {
    expect(
      nextEnvironmentName(daytona, {
        daytona: createEnvironmentProfile(daytona),
        "daytona-2": createEnvironmentProfile(daytona),
      }),
    ).toBe("daytona-3");
  });
});
