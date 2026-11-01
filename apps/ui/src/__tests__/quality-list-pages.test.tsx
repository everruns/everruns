import { fireEvent, render, screen, within } from "@testing-library/react";
import type { ReactNode } from "react";
import EvalsPage from "@/app/(main)/evals/page";
import ObserversPageClient from "@/app/(main)/observers/observers-page-client";
import type { Eval, Observer } from "@/lib/api/types";

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
    <a href={href} {...props}>
      {children}
    </a>
  ),
}));

let pathname = "/evals";
let search = "";
const mockReplace = jest.fn();
jest.mock("next/navigation", () => ({
  usePathname: () => pathname,
  useSearchParams: () => new URLSearchParams(search),
  useRouter: () => ({ replace: mockReplace, push: jest.fn() }),
}));

const mockUseEvals = jest.fn();
const mockUseAgents = jest.fn();
const mockUseObservers = jest.fn();

jest.mock("@/hooks", () => ({
  useEvals: (...args: unknown[]) => mockUseEvals(...args),
  useAgents: (...args: unknown[]) => mockUseAgents(...args),
  useObservers: (...args: unknown[]) => mockUseObservers(...args),
  useAllEvalExperiments: () => ({ data: [] }),
  useEvalRunsMany: () => new Map(),
  useCreateEvalExperiment: () => ({ mutateAsync: jest.fn(), isPending: false }),
  usePageTitle: () => undefined,
}));

jest.mock("@/hooks/use-health-issues", () => ({
  useEvalFindings: () => ({
    data: {
      data: [
        {
          id: "issue_1",
          code: "session.suspected_abuse",
          status: "open",
          title: "Session looks like abuse",
          body: "",
          missing_scopes: [],
          first_detected_at: "2026-11-01T00:00:00Z",
          last_checked_at: "2026-11-01T00:00:00Z",
          stale: false,
          href: "/health/issue_1",
        },
      ],
    },
  }),
}));

jest.mock("@/providers/feature-flags-provider", () => ({
  useFeatureFlag: () => true,
}));

const activeEval: Eval = {
  id: "eval_active",
  name: "Active eval",
  tags: [],
  status: "active",
  case_count: 1,
  created_at: "2026-09-14T00:00:00Z",
  updated_at: "2026-09-14T00:00:00Z",
};

const archivedEval: Eval = {
  ...activeEval,
  id: "eval_archived",
  name: "Archived eval",
  status: "archived",
};

const activeObserver: Observer = {
  id: "observer_active",
  name: "Active observer",
  match: {},
  sampling_rate: 0.1,
  scorers: [],
  status: "active",
  created_at: "2026-09-14T00:00:00Z",
  updated_at: "2026-09-14T00:00:00Z",
};

const archivedObserver: Observer = {
  ...activeObserver,
  id: "observer_archived",
  name: "Archived observer",
  status: "archived",
};

beforeEach(() => {
  jest.clearAllMocks();
  mockUseAgents.mockReturnValue({ data: [] });
});

it("opens the Evals section on the overview and switches tabs through the URL", () => {
  pathname = "/evals";
  search = "";
  mockUseEvals.mockReturnValue({ data: [activeEval, archivedEval], isLoading: false, error: null });
  mockUseObservers.mockReturnValue({ data: [activeObserver] });

  render(<EvalsPage />);

  expect(screen.getByRole("navigation", { name: "Breadcrumb" })).toHaveTextContent("Quality/Evals");
  expect(screen.getByRole("heading", { name: "Evals" })).toBeInTheDocument();
  expect(
    screen.getByText("How your agents do on test datasets and on real traffic."),
  ).toBeInTheDocument();
  expect(screen.getByRole("tab", { name: "Overview" })).toHaveAttribute("aria-selected", "true");
  // The abuse finding is the one thing that needs a look.
  expect(screen.getByText("Session looks like abuse")).toBeInTheDocument();
  expect(screen.getByText("High")).toBeInTheDocument();
  expect(mockUseEvals).toHaveBeenCalledWith({ includeArchived: true });

  fireEvent.click(screen.getByRole("tab", { name: "Datasets" }));
  expect(mockReplace).toHaveBeenCalledWith("/evals?tab=datasets", { scroll: false });
});

it("lists active and archived datasets on the Datasets tab", () => {
  pathname = "/evals";
  search = "tab=datasets";
  mockUseEvals.mockReturnValue({ data: [activeEval, archivedEval], isLoading: false, error: null });
  mockUseObservers.mockReturnValue({ data: [] });

  render(<EvalsPage />);

  expect(within(screen.getByRole("tab", { name: "Active" })).getByText("1")).toBeInTheDocument();
  expect(within(screen.getByRole("tab", { name: "Archived" })).getByText("1")).toBeInTheDocument();
  expect(screen.getByText("Active eval")).toBeInTheDocument();
  expect(screen.queryByText("Archived eval")).not.toBeInTheDocument();

  fireEvent.click(screen.getByRole("tab", { name: "Archived" }));

  expect(screen.getByText("Archived eval")).toBeInTheDocument();
  expect(screen.queryByText("Active eval")).not.toBeInTheDocument();
});

it("uses the shared masthead and status tabs for observers", () => {
  pathname = "/observers";
  mockUseObservers.mockReturnValue({
    data: [activeObserver, archivedObserver],
    isLoading: false,
    error: null,
  });

  render(<ObserversPageClient />);

  expect(screen.getByRole("navigation", { name: "Breadcrumb" })).toHaveTextContent(
    "Quality/Observers",
  );
  expect(screen.getByRole("heading", { name: "Observers" })).toBeInTheDocument();
  expect(
    screen.getByText(
      "Score production sessions asynchronously with sampling rules and evaluators.",
    ),
  ).toBeInTheDocument();
  const observersMasthead = document.querySelector<HTMLElement>('[data-slot="page-masthead"]');
  expect(observersMasthead).not.toBeNull();
  expect(within(observersMasthead!).queryByText("2")).not.toBeInTheDocument();
  expect(within(screen.getByRole("tab", { name: "Active" })).getByText("1")).toBeInTheDocument();
  expect(within(screen.getByRole("tab", { name: "Archived" })).getByText("1")).toBeInTheDocument();
  expect(mockUseObservers).toHaveBeenCalledWith({
    includeArchived: true,
    enabled: true,
  });
  expect(screen.getByText("Active observer")).toBeInTheDocument();
  expect(screen.queryByText("Archived observer")).not.toBeInTheDocument();

  fireEvent.click(screen.getByRole("tab", { name: "Archived" }));

  expect(screen.getByText("Archived observer")).toBeInTheDocument();
  expect(screen.queryByText("Active observer")).not.toBeInTheDocument();
});
