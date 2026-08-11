import { render, screen, act, fireEvent } from "@testing-library/react";
import { Suspense, type ReactNode } from "react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import EvalRunDetailPage from "@/app/(main)/evals/[evalId]/runs/[runId]/page";
import type { Eval, EvalRun } from "@/lib/api/types";

// The Share button (rendered in the run header) queries share status; stub the
// share API so it resolves cleanly under the test QueryClient.
jest.mock("@/lib/api/evals", () => ({
  getRunShareStatus: jest.fn().mockResolvedValue({ active: false }),
  createRunShare: jest.fn(),
  revokeRunShare: jest.fn(),
}));

jest.mock("next/link", () => ({
  __esModule: true,
  default: ({
    children,
    href,
    ...props
  }: {
    children: ReactNode;
    href: string;
    [key: string]: unknown;
  }) => (
    // eslint-disable-next-line @next/next/no-html-link-for-pages
    <a href={href} {...props}>
      {children}
    </a>
  ),
}));

const mockUseEval = jest.fn();
const mockUseEvalRun = jest.fn();
const mockUseCancelEvalRun = jest.fn();

jest.mock("@/hooks", () => ({
  useEval: (...args: unknown[]) => mockUseEval(...args),
  useEvalRun: (...args: unknown[]) => mockUseEvalRun(...args),
  useCancelEvalRun: (...args: unknown[]) => mockUseCancelEvalRun(...args),
  usePageTitle: () => undefined,
}));

const mockEval: Eval = {
  id: "eval_123",
  name: "Session link eval",
  tags: [],
  status: "active",
  case_count: 1,
  created_at: "2026-04-19T10:00:00Z",
  updated_at: "2026-04-19T10:00:00Z",
};

const mockRun: EvalRun = {
  id: "evalrun_123",
  status: "completed",
  triggered_by: "user",
  results: [
    {
      id: "evalresult_123",
      eval_case_id: "evalcase_123",
      case_name: "Case with session",
      session_id: "session_123",
      status: "passed",
      created_at: "2026-04-19T10:00:00Z",
      updated_at: "2026-04-19T10:00:00Z",
    },
  ],
  created_at: "2026-04-19T10:00:00Z",
  updated_at: "2026-04-19T10:01:00Z",
};

async function renderWithSuspense(params: { evalId: string; runId: string }) {
  const paramsPromise = Promise.resolve(params);
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });

  await act(async () => {
    render(
      <QueryClientProvider client={queryClient}>
        <Suspense fallback={<div>Loading...</div>}>
          <EvalRunDetailPage params={paramsPromise} />
        </Suspense>
      </QueryClientProvider>,
    );
    await paramsPromise;
  });
}

describe("EvalRunDetailPage", () => {
  beforeEach(() => {
    jest.clearAllMocks();
    mockUseEval.mockReturnValue({ data: mockEval });
    mockUseEvalRun.mockReturnValue({ data: mockRun, isLoading: false });
    mockUseCancelEvalRun.mockReturnValue({
      mutateAsync: jest.fn(),
      isPending: false,
    });
  });

  it("opens result session links in a new tab", async () => {
    await renderWithSuspense({ evalId: "eval_123", runId: "evalrun_123" });

    const link = screen.getByLabelText("Open session for Case with session in new tab");
    expect(link).toHaveAttribute("href", "/sessions/session_123/transcript");
    expect(link).toHaveAttribute("target", "_blank");
    expect(link).toHaveAttribute("rel", "noopener noreferrer");
  });

  it("shows attribution and an expandable transcript for an external run", async () => {
    const externalRun: EvalRun = {
      id: "evalrun_ext",
      status: "completed",
      source: "external",
      attribution: { system: "mira", version: "0.1.0", run_id: "20260101T000000Z-abcd" },
      triggered_by: "import:mira",
      results: [
        {
          id: "evalresult_ext",
          eval_case_id: "evalcase_ext",
          case_name: "imported case",
          target_snapshot: { type: "external", provider: "anthropic", model: "opus" },
          status: "passed",
          scores: [{ scorer: "contains", value: 1, pass: true, reason: "ok" }],
          metadata: { transcript: { final_response: "the imported answer" } },
          created_at: "2026-04-19T10:00:00Z",
          updated_at: "2026-04-19T10:00:00Z",
        },
      ],
      created_at: "2026-04-19T10:00:00Z",
      updated_at: "2026-04-19T10:01:00Z",
    };
    mockUseEvalRun.mockReturnValue({ data: externalRun, isLoading: false });

    await renderWithSuspense({ evalId: "eval_123", runId: "evalrun_ext" });

    // Attribution badge and the external target label render.
    expect(screen.getByText("via mira 0.1.0")).toBeInTheDocument();
    expect(screen.getByText("anthropic/opus")).toBeInTheDocument();
    // Named score renders by scorer name, not positional index.
    expect(screen.getByText("contains: 1.0")).toBeInTheDocument();

    // Transcript is hidden until the row is expanded.
    expect(screen.queryByText("the imported answer")).not.toBeInTheDocument();
    act(() => {
      fireEvent.click(screen.getByLabelText("Show transcript"));
    });
    expect(screen.getByText("the imported answer")).toBeInTheDocument();
  });

  it("does not link unsafe attribution URLs", async () => {
    const externalRun: EvalRun = {
      id: "evalrun_ext_unsafe",
      status: "completed",
      source: "external",
      attribution: { system: "mira", url: "javascript:alert(1)" },
      triggered_by: "import:mira",
      results: [],
      created_at: "2026-04-19T10:00:00Z",
      updated_at: "2026-04-19T10:01:00Z",
    };
    mockUseEvalRun.mockReturnValue({ data: externalRun, isLoading: false });

    await renderWithSuspense({ evalId: "eval_123", runId: "evalrun_ext_unsafe" });

    expect(screen.getByText("via mira").closest("a")).toBeNull();
  });

  it("defaults a multi-target run to the matrix view", async () => {
    const mkResult = (id: string, provider: string, model: string, value: number) => ({
      id,
      eval_case_id: "evalcase_add",
      case_name: "add",
      target_snapshot: { type: "external" as const, provider, model },
      status: value >= 0.5 ? ("passed" as const) : ("failed" as const),
      scores: [{ scorer: "contains", value, pass: value >= 0.5, reason: "" }],
      created_at: "2026-04-19T10:00:00Z",
      updated_at: "2026-04-19T10:00:00Z",
    });
    const matrixRun: EvalRun = {
      id: "evalrun_matrix",
      status: "completed",
      source: "external",
      attribution: { system: "mira" },
      triggered_by: "import:mira",
      results: [mkResult("r1", "anthropic", "opus", 1), mkResult("r2", "openai", "gpt", 0)],
      created_at: "2026-04-19T10:00:00Z",
      updated_at: "2026-04-19T10:01:00Z",
    };
    mockUseEvalRun.mockReturnValue({ data: matrixRun, isLoading: false });

    await renderWithSuspense({ evalId: "eval_123", runId: "evalrun_matrix" });

    // Matrix column headers are the two distinct targets, and the case is a row.
    expect(screen.getByText("anthropic/opus")).toBeInTheDocument();
    expect(screen.getByText("openai/gpt")).toBeInTheDocument();
    expect(screen.getByText("add")).toBeInTheDocument();
    // Cell scores render (1.00 for the passing target, 0.00 for the failing one).
    expect(screen.getByText("1.00")).toBeInTheDocument();
    expect(screen.getByText("0.00")).toBeInTheDocument();
    // The view toggle is offered.
    expect(screen.getByRole("button", { name: "Table" })).toBeInTheDocument();
  });
});
