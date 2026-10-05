import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen, waitFor } from "@testing-library/react";
import { SessionSandboxPanel } from "@/components/session/session-sandbox-panel";
import { api } from "@/lib/api/client";
import type { SessionSandbox } from "@/lib/api/sandbox-templates";

jest.mock("@/lib/api/client", () => ({
  api: { get: jest.fn() },
}));

const mockedGet = api.get as jest.Mock;

function sandbox(overrides: Partial<SessionSandbox> = {}): SessionSandbox {
  return {
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
    ...overrides,
  };
}

function renderPanel() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <SessionSandboxPanel sessionId="session_1" />
    </QueryClientProvider>,
  );
}

describe("SessionSandboxPanel", () => {
  afterEach(() => {
    mockedGet.mockReset();
  });

  it("names the target and its provider", async () => {
    mockedGet.mockResolvedValue({ data: sandbox() });

    renderPanel();

    expect(await screen.findByText("Virtual filesystem")).toBeInTheDocument();
    expect(screen.getByText("bashkit")).toBeInTheDocument();
  });

  it("says a bashkit session cannot run native processes", async () => {
    mockedGet.mockResolvedValue({ data: sandbox() });

    renderPanel();

    // The row exists and reads "no", so an operator learns a build will fail
    // here before running one.
    const row = (await screen.findByText("Native processes")).closest("div");
    expect(row).toHaveTextContent("no");
  });

  it("marks an uncontained sandbox rather than dressing it up", async () => {
    mockedGet.mockResolvedValue({
      data: sandbox({
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
        source_capability: undefined,
      }),
    });

    renderPanel();

    expect(await screen.findByText("This machine")).toBeInTheDocument();
    expect(screen.getByText("Uncontained")).toBeInTheDocument();
    expect(screen.getByText("Recovery: Not recoverable")).toBeInTheDocument();
  });

  it("explains a session that runs no commands at all", async () => {
    mockedGet.mockResolvedValue({
      data: sandbox({ target: undefined, source_capability: undefined }),
    });

    renderPanel();

    expect(await screen.findByText("No compute")).toBeInTheDocument();
    expect(
      screen.getByText("This session runs no commands. It reads and writes files only."),
    ).toBeInTheDocument();
  });

  it("does not warn about containment when nothing runs", async () => {
    mockedGet.mockResolvedValue({
      data: sandbox({ target: undefined, source_capability: undefined }),
    });

    renderPanel();

    await screen.findByText("No compute");
    // "Uncontained" would read as a warning about a session that cannot run
    // anything at all.
    expect(screen.queryByText("Uncontained")).not.toBeInTheDocument();
  });

  it("renders nothing when the sandbox cannot be read", async () => {
    mockedGet.mockRejectedValue(new Error("boom"));

    const { container } = renderPanel();

    await waitFor(() => expect(container).toBeEmptyDOMElement());
  });
});
