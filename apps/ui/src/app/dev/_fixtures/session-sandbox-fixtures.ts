// Sandbox panel fixtures.
//
// The four sandbox states a session can actually have today, plus the one that
// matters most for review: a real machine, which contains nothing and recovers
// nothing, and has to look different from the contained ones.

import type { SessionSandbox } from "@/lib/api/sandbox-templates";

export interface SessionSandboxScenario {
  name: string;
  sessionId: string;
  sandbox: SessionSandbox;
}

export const sessionSandboxScenarios: SessionSandboxScenario[] = [
  {
    name: "Bashkit — no native binaries",
    sessionId: "session_dev_bashkit",
    sandbox: {
      target: { kind: "vfs", provider: "bashkit" },
      containment: { level: "isolated", network: "deny" },
      durability: "checkpointed",
      capabilities: {
        native_processes: false,
        packages: false,
        pty: false,
        ports: false,
        portable_checkpoint: true,
        network_enforced: true,
      },
      resolved_from: "capabilities",
      source_capability: "bashkit_shell",
    },
  },
  {
    name: "Daytona with a recovery volume",
    sessionId: "session_dev_daytona",
    sandbox: {
      target: { kind: "managed", provider: "daytona" },
      containment: { level: "isolated", network: "deny" },
      durability: "checkpointed",
      capabilities: {
        native_processes: true,
        packages: true,
        pty: true,
        ports: true,
        portable_checkpoint: true,
        network_enforced: true,
      },
      resolved_from: "capabilities",
      source_capability: "session_sandbox",
    },
  },
  {
    name: "This machine — uncontained",
    sessionId: "session_dev_host",
    sandbox: {
      target: { kind: "host" },
      containment: { level: "none", network: "allow" },
      durability: "none",
      capabilities: {
        native_processes: true,
        packages: true,
        pty: true,
        ports: true,
        portable_checkpoint: false,
        network_enforced: false,
      },
      resolved_from: "capabilities",
    },
  },
  {
    name: "Files only — no compute",
    sessionId: "session_dev_files",
    sandbox: {
      containment: { level: "none", network: "deny" },
      durability: "checkpointed",
      capabilities: {
        native_processes: false,
        packages: false,
        pty: false,
        ports: false,
        portable_checkpoint: true,
        network_enforced: true,
      },
      resolved_from: "capabilities",
    },
  },
];
