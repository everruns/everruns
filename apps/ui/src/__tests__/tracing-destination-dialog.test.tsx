import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { TracingDestinationDialog } from "@/components/tracing/destination-dialog";
import { saveTracingDestination, type TracingDestination } from "@/lib/api/tracing";
import type { ReactNode } from "react";

jest.mock("@/lib/api/tracing", () => ({ saveTracingDestination: jest.fn() }));
jest.mock("@/hooks/use-tracing", () => ({ useRefreshTracing: () => async () => {} }));
jest.mock("@/providers/locale-provider", () => ({ useUiTranslations: () => (s: string) => s }));
jest.mock("@/components/ui/dialog", () => ({
  Dialog: ({ children }: { children: ReactNode }) => <>{children}</>,
  DialogContent: ({ children }: { children: ReactNode }) => <section>{children}</section>,
  DialogHeader: ({ children }: { children: ReactNode }) => <>{children}</>,
  DialogTitle: ({ children }: { children: ReactNode }) => <h1>{children}</h1>,
  DialogDescription: ({ children }: { children: ReactNode }) => <p>{children}</p>,
  DialogFooter: ({ children }: { children: ReactNode }) => <>{children}</>,
}));
jest.mock("@/components/ui/select", () => ({
  Select: ({
    children,
    value,
    onValueChange,
    disabled,
  }: {
    children: ReactNode;
    value: string;
    onValueChange: (v: string) => void;
    disabled?: boolean;
  }) => (
    <select
      aria-label="Provider"
      value={value}
      disabled={disabled}
      onChange={(e) => onValueChange(e.target.value)}
    >
      {children}
    </select>
  ),
  SelectTrigger: () => null,
  SelectValue: () => null,
  SelectContent: ({ children }: { children: ReactNode }) => <>{children}</>,
  SelectItem: ({ children, value }: { children: ReactNode; value: string }) => (
    <option value={value}>{children}</option>
  ),
}));
const save = jest.mocked(saveTracingDestination);
beforeEach(() => {
  save.mockReset();
  save.mockResolvedValue({} as TracingDestination);
});
function fill(label: string, value: string) {
  fireEvent.change(screen.getByLabelText(label), { target: { value } });
}
function submit() {
  fireEvent.submit(screen.getByRole("button", { name: "Create destination" }).closest("form")!);
}

it("creates Langfuse metadata-only destination with write-only Basic credentials", async () => {
  const onClose = jest.fn();
  render(<TracingDestinationDialog onClose={onClose} onCreated={() => {}} />);
  fill("Provider", "langfuse");
  fill("Destination name", "Private traces");
  fill("Public key", "pk-example");
  fill("Secret key", "sk-example");
  submit();
  await waitFor(() => expect(onClose).toHaveBeenCalled());
  expect(save).toHaveBeenCalledWith(
    expect.objectContaining({
      config: expect.objectContaining({
        provider: "langfuse",
        capture_content: false,
        capture_reasoning: false,
      }),
      secrets: { headers: { Authorization: `Basic ${btoa("pk-example:sk-example")}` } },
    }),
    undefined,
  );
  expect(JSON.stringify(save.mock.calls[0][0].config)).not.toContain("sk-example");
});
it("resets credentials on provider switch and sends LangSmith project separately", async () => {
  render(<TracingDestinationDialog onClose={() => {}} onCreated={() => {}} />);
  fill("Provider", "langfuse");
  fill("Public key", "pk-old");
  fill("Secret key", "sk-old");
  fill("Provider", "langsmith");
  expect(screen.getByLabelText("API key")).toHaveValue("");
  fill("Destination name", "Smith");
  fill("Project name", " production ");
  fill("API key", "ls-example");
  submit();
  await waitFor(() => expect(save).toHaveBeenCalled());
  expect(save.mock.calls[0][0]).toMatchObject({
    config: { provider: "langsmith", project_name: "production", project_id: null },
    secrets: { headers: { "x-api-key": "ls-example" } },
  });
  expect(JSON.stringify(save.mock.calls[0][0])).not.toContain("sk-old");
});
it("preserves existing encrypted credentials when editing ordinary settings", async () => {
  const destination: TracingDestination = {
    id: "test",
    revision: 3,
    config: {
      name: "Smith",
      provider: "langsmith",
      endpoint: "https://api.smith.langchain.com/otel/v1/traces",
      project_name: "prod",
      project_id: null,
      capture_content: false,
      capture_reasoning: false,
      enabled: true,
    },
    last_accepted_at: null,
    last_attempt_at: null,
    last_error: null,
    dropped_events: 0,
    organization_default: false,
    agent_selections: 0,
  };
  render(
    <TracingDestinationDialog destination={destination} onClose={() => {}} onCreated={() => {}} />,
  );
  expect(screen.queryByLabelText("API key")).not.toBeInTheDocument();
  fill("Destination name", "Updated");
  fireEvent.submit(screen.getByRole("button", { name: "Save changes" }).closest("form")!);
  await waitFor(() => expect(save).toHaveBeenCalled());
  expect(save.mock.calls[0][0]).not.toHaveProperty("secrets");
});
it("cancel does not write", () => {
  const onClose = jest.fn();
  render(<TracingDestinationDialog onClose={onClose} onCreated={() => {}} />);
  fill("Provider", "langsmith");
  fill("API key", "example");
  fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
  expect(onClose).toHaveBeenCalled();
  expect(save).not.toHaveBeenCalled();
});

it("requires fresh credentials when an existing destination endpoint changes", async () => {
  const destination: TracingDestination = {
    id: "test",
    revision: 3,
    config: {
      name: "Smith",
      provider: "langsmith",
      endpoint: "https://api.smith.langchain.com/otel/v1/traces",
      project_name: "prod",
      project_id: null,
      capture_content: false,
      capture_reasoning: false,
      enabled: true,
    },
    last_accepted_at: null,
    last_attempt_at: null,
    last_error: null,
    dropped_events: 0,
    organization_default: false,
    agent_selections: 0,
  };
  render(
    <TracingDestinationDialog destination={destination} onClose={() => {}} onCreated={() => {}} />,
  );
  fill("Endpoint", "https://eu.api.smith.langchain.com/otel/v1/traces");
  expect(screen.getByLabelText("API key")).toHaveValue("");
  expect(screen.getByLabelText("API key")).toBeRequired();
  fill("API key", "ls-new-region");
  fireEvent.submit(screen.getByRole("button", { name: "Save changes" }).closest("form")!);
  await waitFor(() => expect(save).toHaveBeenCalled());
  expect(save.mock.calls[0][0]).toMatchObject({
    secrets: { headers: { "x-api-key": "ls-new-region" } },
  });
});

it("uses agent-based LangSmith routing without conflicting project settings", async () => {
  render(<TracingDestinationDialog onClose={() => {}} onCreated={() => {}} />);
  fill("Provider", "langsmith");
  fill("Project name", "old-project");
  fireEvent.click(screen.getByRole("checkbox", { name: "Agent-based LangSmith workspace" }));
  expect(screen.queryByLabelText("Project name")).not.toBeInTheDocument();
  fill("Destination name", "Smith agent");
  fill("LangSmith agent ID", "my-agent");
  fill("LangSmith environment", "staging");
  fill("API key", "ls-example");
  submit();
  await waitFor(() => expect(save).toHaveBeenCalled());
  expect(save.mock.calls[0][0].config).toMatchObject({
    project_name: null,
    langsmith_agent_id: "my-agent",
    langsmith_agent_environment: "staging",
  });
});
