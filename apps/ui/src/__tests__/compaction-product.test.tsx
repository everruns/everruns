import { fireEvent, render, screen } from "@testing-library/react";
import { CompactionConfigEditor } from "@/components/agents/compaction-config-editor";
import {
  SessionContextButton,
  SessionContextDetails,
} from "@/components/session/session-context-button";
import { ContextCard } from "@/components/session/cost/context-card";
import { CompactionNotice } from "@/components/chat/compaction-notice";
import type { SessionContextReport } from "@/lib/api/types";
import { useSessionContextReport } from "@/hooks/use-sessions";

jest.mock("@/hooks/use-sessions", () => ({ useSessionContextReport: jest.fn() }));
const useReport = jest.mocked(useSessionContextReport);
const report = {
  session_id: "session-test",
  model: "test-model",
  measurement_available: false,
  estimated_input_tokens: 0,
  sections: [],
  contributions: [],
  compaction: {
    enabled: true,
    strategy: "auto",
    proactive: true,
    context_budget_tokens: 64000,
    history_retrieval_available: true,
    checkpoint_store_available: true,
    budget_percent: 0.85,
    last_event: null,
    checkpoint: {
      id: "checkpoint-1",
      kind: "portable_summary",
      source_sequence: 480,
      provider: null,
      model: null,
    },
  },
} as SessionContextReport;

beforeEach(() => useReport.mockReset());

test("configuration edits preserve advanced tuning and authored instructions", () => {
  const onChange = jest.fn();
  const config = {
    cost_control: { enabled: false },
    summarization: { preserve: ["decisions"], instructions: "Keep approval boundaries." },
  };
  render(<CompactionConfigEditor config={config} onChange={onChange} />);
  fireEvent.change(screen.getByRole("spinbutton", { name: "Working history budget" }), {
    target: { value: "8192" },
  });
  expect(onChange).toHaveBeenLastCalledWith({ ...config, context_budget_tokens: 8192 });
  fireEvent.click(screen.getByText("Advanced"));
  fireEvent.change(screen.getByRole("textbox", { name: "Summary model" }), {
    target: { value: "provider-model" },
  });
  expect(onChange).toHaveBeenLastCalledWith({
    ...config,
    summarization: { ...config.summarization, model: "provider-model" },
  });
  fireEvent.change(screen.getByRole("spinbutton", { name: "Working history budget" }), {
    target: { value: "2048" },
  });
  expect(onChange).toHaveBeenLastCalledWith({ ...config, context_budget_tokens: 2048 });
});

test("context inspection is lazy and shows policy before the first model request", () => {
  useReport.mockReturnValue({ data: report, isLoading: false, isError: false } as ReturnType<
    typeof useSessionContextReport
  >);
  render(<SessionContextButton sessionId="session-test" />);
  expect(useReport).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("button", { name: "Context" }));
  expect(useReport).toHaveBeenCalledWith("session-test");
  expect(screen.getByText("No recent model context measurement.")).toBeVisible();
  expect(screen.getByText("Automatic")).toBeVisible();
  expect(screen.getByText("Portable summary")).toBeVisible();
  expect(screen.getByText("History through event 480")).toBeVisible();
});

test("missing measurements never present an empty context as zero percent full", () => {
  render(<ContextCard report={{ ...report, context_window_tokens: 200000 }} />);
  expect(screen.getByText("No recent model context measurement.")).toBeVisible();
  expect(screen.queryByText("0% full")).not.toBeInTheDocument();
});

test("unreported remote storage is distinct from having no saved checkpoint", () => {
  render(
    <SessionContextDetails
      report={{
        ...report,
        compaction: {
          ...report.compaction!,
          checkpoint: null,
          checkpoint_store_available: null,
          last_event: {
            state: "completed",
            strategy: "portable_summary",
            checkpoint_id: "remote-checkpoint",
            source_sequence: 900,
            tokens_before: 90000,
            tokens_after: 15000,
            duration_ms: 1000,
          },
        },
      }}
    />,
  );
  expect(screen.getByText("Checkpoint storage has not been reported by this host.")).toBeVisible();
  expect(screen.queryByText("No checkpoint has been saved yet.")).not.toBeInTheDocument();
  expect(screen.getByText(/Checkpoint saved/)).toBeVisible();
  expect(screen.getByText(/History through event 900/)).toBeVisible();
});

test("lifecycle states use safe progress and failure copy", () => {
  const { rerender } = render(<CompactionNotice />);
  expect(screen.getByRole("status")).toHaveTextContent("Compacting context…");
  rerender(<CompactionNotice failed />);
  expect(screen.getByRole("alert")).toHaveTextContent(
    "Compaction failed. Previous history was retained.",
  );
});

test("a legacy scoped summary is not advertised as portable across models", () => {
  render(
    <SessionContextDetails
      report={{
        ...report,
        compaction: {
          ...report.compaction!,
          checkpoint: {
            id: "legacy",
            kind: "legacy_summary",
            source_sequence: 400,
            provider: "openai",
            model: "model-a",
          },
        },
      }}
    />,
  );
  expect(screen.getByText("Summary", { exact: true })).toBeVisible();
  expect(screen.getByText("openai · model-a")).toBeVisible();
  expect(screen.queryByText("Portable summary", { exact: true })).not.toBeInTheDocument();
});
