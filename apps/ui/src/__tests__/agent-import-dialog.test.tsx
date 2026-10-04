import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { AgentImportDialog } from "@/components/agents/agent-import-dialog";
import { inspectAgentPackage } from "@/lib/api/agents";
import type { Agent } from "@/lib/api/types";

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
      ? { valid: true, diagnostics: [] }
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
  expect(await screen.findByText("/instructions")).toBeInTheDocument();
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
