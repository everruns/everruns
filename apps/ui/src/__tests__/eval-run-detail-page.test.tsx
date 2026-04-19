import { render, screen, act } from "@testing-library/react";
import { Suspense, type ReactNode } from "react";
import EvalRunDetailPage from "@/app/(main)/evals/[evalId]/runs/[runId]/page";
import type { Eval, EvalRun } from "@/lib/api/types";

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

  await act(async () => {
    render(
      <Suspense fallback={<div>Loading...</div>}>
        <EvalRunDetailPage params={paramsPromise} />
      </Suspense>,
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
    expect(link).toHaveAttribute("href", "/sessions/session_123");
    expect(link).toHaveAttribute("target", "_blank");
    expect(link).toHaveAttribute("rel", "noopener noreferrer");
  });
});
