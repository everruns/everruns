import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { PackageChangeReview } from "@/components/agents/agent-package-review";
import { AgentImportDialog } from "@/components/agents/agent-import-dialog";
import { inspectAgentPackage } from "@/lib/api/agents";
import type { Agent, Capability } from "@/lib/api/types";

jest.mock("@/components/chat/streamdown-message", () => ({
  StreamdownMessage: ({ children }: { children: string }) => <div>{children}</div>,
}));

const mutateAsync = jest.fn();
jest.mock("@/lib/api/agents", () => ({ inspectAgentPackage: jest.fn() }));
jest.mock("@/hooks/use-agents", () => ({ useImportAgent: () => ({ mutateAsync }) }));
const inspect = jest.mocked(inspectAgentPackage);
const file = new File(["name = 'triage'"], "agent.toml");
const agent = { id: "agent-test", name: "triage", display_name: "Triage" } as Agent;
beforeEach(() => jest.clearAllMocks());

test("validation errors prevent importing", async () => {
  inspect.mockResolvedValue({
    valid: false,
    diagnostics: [{ path: "model", message: "missing destination model" }],
  });
  render(<AgentImportDialog file={file} agents={[]} onClose={jest.fn()} onImported={jest.fn()} />);
  expect(await screen.findByRole("alert")).toHaveTextContent("model: missing destination model");
  expect(screen.getByRole("button", { name: /^Import$/ })).toBeDisabled();
  expect(mutateAsync).not.toHaveBeenCalled();
});

test("updates validate and show the diff before applying the same file", async () => {
  inspect.mockImplementation(async (_file, operation) =>
    operation === "validate"
      ? {
          valid: true,
          diagnostics: [],
          preview: { name: "triage", instructions: "New", capabilities: {}, files: {} },
        }
      : { changes: [{ path: "/instructions", before: "Old", after: "New" }] },
  );
  mutateAsync.mockResolvedValue(agent);
  const onClose = jest.fn();
  const onImported = jest.fn();
  render(
    <AgentImportDialog file={file} agents={[agent]} onClose={onClose} onImported={onImported} />,
  );
  await waitFor(() => expect(screen.getByLabelText("Import destination")).toBeEnabled());
  fireEvent.change(screen.getByLabelText("Import destination"), { target: { value: "triage" } });
  fireEvent.click(await screen.findByRole("tab", { name: "Changes" }));
  expect(screen.getByText("instructions")).toBeInTheDocument();
  expect(inspect).toHaveBeenCalledWith(file, "validate", "triage");
  expect(inspect).toHaveBeenCalledWith(file, "diff", "triage");
  fireEvent.click(screen.getByRole("button", { name: "Apply changes" }));
  await waitFor(() => expect(mutateAsync).toHaveBeenCalledWith({ file, target: "triage" }));
  expect(onImported).toHaveBeenCalledWith(agent);
  expect(onClose).toHaveBeenCalled();
});

test("import failures remain visible without closing the dialog", async () => {
  inspect.mockResolvedValue({ valid: true, diagnostics: [] });
  mutateAsync.mockRejectedValue(new Error("Destination changed"));
  const onClose = jest.fn();
  render(<AgentImportDialog file={file} agents={[]} onClose={onClose} onImported={jest.fn()} />);
  await waitFor(() => expect(screen.getByRole("button", { name: /^Import$/ })).toBeEnabled());
  fireEvent.click(screen.getByRole("button", { name: /^Import$/ }));
  expect(await screen.findByRole("alert")).toHaveTextContent("Destination changed");
  expect(onClose).not.toHaveBeenCalled();
});

test("new imports preview instructions, destination defaults, root files, skills and channels", async () => {
  inspect.mockResolvedValue({
    valid: true,
    preview: {
      name: "triage",
      display_name: "Triage assistant",
      instructions: "Read the runbook.",
      capabilities: { session_file_system: {} },
      files: {
        "runbook.md": { bytes: 11, is_readonly: false, sha256: "digest" },
        ".agents/skills/investigate/SKILL.md": { bytes: 20, is_readonly: true, sha256: "digest" },
      },
      channels: { chat: { type: "ag_ui", enabled: false, config: {} } },
    },
  });
  render(
    <AgentImportDialog
      file={file}
      agents={[]}
      capabilities={[
        { id: "session_file_system", name: "File System", is_guardrail: true } as Capability,
      ]}
      onClose={jest.fn()}
      onImported={jest.fn()}
    />,
  );
  expect(await screen.findByRole("region", { name: "Agent preview" })).toBeInTheDocument();
  expect(screen.getByText("Read the runbook.")).toBeInTheDocument();
  expect(screen.getByText("File System")).toBeInTheDocument();
  expect(screen.getByText("Guardrail")).toBeInTheDocument();
  expect(screen.getByText("Destination default model")).toBeInTheDocument();
  expect(screen.getByRole("tab", { name: "Agent" })).toHaveAttribute("aria-selected", "true");
  expect(screen.queryByRole("button", { name: "Edit prompt" })).not.toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Source" }));
  expect(screen.getByRole("button", { name: "Rendered" })).toHaveAttribute("aria-pressed", "true");
  fireEvent.click(screen.getByRole("tab", { name: "Files" }));
  expect(screen.queryByRole("region", { name: "Instructions" })).not.toBeInTheDocument();
  expect(screen.getByText("runbook.md")).toBeInTheDocument();
  expect(screen.getByText("11 B · Writable")).toBeInTheDocument();
  expect(screen.getByText("Skills: investigate")).toBeInTheDocument();
  fireEvent.click(screen.getByRole("tab", { name: "Integrations" }));
  expect(screen.getByText("Disabled for new channels")).toBeInTheDocument();
  fireEvent.click(screen.getByRole("tab", { name: "Settings" }));
  expect(screen.getByText("Parallel tool calls")).toBeInTheDocument();
  expect(screen.getByRole("button", { name: /^Import$/ })).toBeEnabled();
  expect(mutateAsync).not.toHaveBeenCalled();
});

test("dependency failure still shows parsed preview but prevents applying", async () => {
  inspect.mockResolvedValue({
    valid: false,
    diagnostics: [{ path: "dependencies", message: "Model is not enabled" }],
    preview: { name: "triage", instructions: "Investigate.", capabilities: {}, files: {} },
  });
  render(<AgentImportDialog file={file} agents={[]} onClose={jest.fn()} onImported={jest.fn()} />);
  expect(await screen.findByRole("alert")).toHaveTextContent("Model is not enabled");
  expect(screen.getByRole("region", { name: "Agent preview" })).toHaveTextContent("Investigate.");
  expect(screen.getByRole("button", { name: /^Import$/ })).toBeDisabled();
});

test("same-size file changes distinguish contents from permissions without showing digests", () => {
  render(
    <PackageChangeReview
      changes={[
        {
          path: "/files/runbook.md",
          before: { bytes: 11, is_readonly: true, sha256: "old-digest" },
          after: { bytes: 11, is_readonly: true, sha256: "new-digest" },
        },
        {
          path: "/files/data~1example.csv",
          before: { bytes: 20, is_readonly: true, sha256: "same-digest" },
          after: { bytes: 20, is_readonly: false, sha256: "same-digest" },
        },
      ]}
    />,
  );
  expect(screen.getByText("files › data/example.csv")).toBeInTheDocument();
  expect(screen.getByText(/Contents changed/)).toBeInTheDocument();
  expect(screen.getByText(/Permissions changed/)).toBeInTheDocument();
  expect(screen.queryByText(/old-digest|new-digest|same-digest/)).not.toBeInTheDocument();
});
