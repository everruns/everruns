import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { SandboxTemplateBuildPanel } from "@/components/sandboxes/sandbox-template-build-panel";
import { ConfirmationProvider } from "@/components/ui/confirmation-provider";
import { api } from "@/lib/api/client";
import type { SandboxTemplate, SandboxTemplateBuild } from "@/lib/api/types";

jest.mock("@/hooks", () => ({
  useOrganizationConnections: () => ({
    data: [{ id: "org-account", name: "Snapshot Builder demo", provider: "daytona" }],
  }),
}));
jest.mock("@/lib/api/client", () => ({ api: { get: jest.fn(), post: jest.fn() } }));
const get = api.get as jest.Mock;
const post = api.post as jest.Mock;
const template: SandboxTemplate = {
  id: "sbxtpl_test",
  name: "dev",
  display_name: "Development",
  is_managed: false,
  status: "active",
  created_at: "2026-10-28T00:00:00Z",
  updated_at: "2026-10-28T00:00:00Z",
  current_revision: {
    id: "sbxtplrev_source",
    sandbox_template_id: "sbxtpl_test",
    revision: 1,
    created_at: "2026-10-28T00:00:00Z",
    spec: {
      target: {
        kind: "managed",
        provider: "daytona",
        options: {},
        credential: { source: "organization", connection_id: "org-account" },
      },
      containment: {
        level: "isolated",
        network: { mode: "deny" },
        filesystem: {},
        escalation: "never",
      },
      lifecycle: {},
      bootstrap: {},
    },
  },
};
function build(overrides: Partial<SandboxTemplateBuild> = {}): SandboxTemplateBuild {
  return {
    id: "sbxbuild_test",
    sandbox_template_id: template.id,
    request: {
      source_revision_id: template.current_revision.id,
      install_script: "echo install",
      timeout_seconds: 600,
    },
    source_spec: template.current_revision.spec,
    status: "ready",
    step: "ready",
    logs: "ready",
    snapshot_name: "baked",
    snapshot_id: "snapshot-test",
    cancel_requested: false,
    cleanup_pending: false,
    created_at: template.created_at,
    updated_at: template.updated_at,
    ...overrides,
  };
}
function panel(dirty = false) {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return render(
    <QueryClientProvider client={client}>
      <ConfirmationProvider>
        <SandboxTemplateBuildPanel template={template} dirty={dirty} />
      </ConfirmationProvider>
    </QueryClientProvider>,
  );
}
afterEach(() => jest.resetAllMocks());

it("submits the exact authored script against the saved revision and build policy", async () => {
  get.mockResolvedValue({ data: [] });
  post.mockResolvedValue({ data: build({ status: "queued" }) });
  panel();
  await waitFor(() => expect(get).toHaveBeenCalled());
  expect(screen.getByText("Snapshot Builder demo")).toBeInTheDocument();
  const script = "set -eu\nprintf '%s' 'tool $(literal)'\n";
  fireEvent.change(screen.getByLabelText("Install script"), { target: { value: script } });
  fireEvent.click(screen.getByRole("button", { name: "Build snapshot" }));
  await waitFor(() =>
    expect(post).toHaveBeenCalledWith(
      `/v1/sandbox-templates/${template.id}/builds`,
      expect.objectContaining({
        source_revision_id: template.current_revision.id,
        install_script: script,
        build_network: { mode: "deny" },
      }),
    ),
  );
  expect(template.current_revision.spec.containment?.network).toEqual({ mode: "deny" });
});

it("requires saved configuration before spending provider resources", async () => {
  get.mockResolvedValue({ data: [] });
  panel(true);
  fireEvent.change(screen.getByLabelText("Install script"), { target: { value: "echo ready" } });
  expect(screen.getByRole("button", { name: "Build snapshot" })).toBeDisabled();
  expect(screen.getByText("Save your configuration changes before building.")).toBeInTheDocument();
});

it("renders notes as inert data and prevents applying a stale build", async () => {
  get.mockResolvedValue({
    data: [
      build({
        software_notes: '<img src=x onerror="alert(1)">',
        request: {
          source_revision_id: "older",
          install_script: "echo install",
          timeout_seconds: 600,
        },
      }),
    ],
  });
  const view = panel();
  expect(await screen.findByText('<img src=x onerror="alert(1)">')).toBeInTheDocument();
  expect(view.container.querySelector("img")).toBeNull();
  expect(screen.getByRole("button", { name: "Save new revision" })).toBeDisabled();
  expect(
    screen.getByText("The source revision changed. Build again from the current template."),
  ).toBeInTheDocument();
});

it("requests cancellation without treating it as completed cleanup", async () => {
  get.mockResolvedValue({
    data: [build({ status: "running", step: "installing", snapshot_id: undefined })],
  });
  post.mockResolvedValue({ data: build({ status: "running", cancel_requested: true }) });
  panel();
  fireEvent.click(await screen.findByRole("button", { name: "Cancel build" }));
  await waitFor(() =>
    expect(post).toHaveBeenCalledWith("/v1/sandbox-template-builds/sbxbuild_test/cancel"),
  );
  expect(screen.queryByRole("button", { name: "Save new revision" })).not.toBeInTheDocument();
});
