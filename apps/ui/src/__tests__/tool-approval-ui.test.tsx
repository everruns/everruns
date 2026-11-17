import { useState } from "react";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { ToolApprovalConfigEditor } from "@/components/agents/tool-approval-config-editor";
import {
  ToolReviewBadge,
  ToolReviewBlockReason,
  ToolReviewsContext,
} from "@/components/chat/tool-review-mark";
import { ToolApprovalRequests } from "@/components/chat/tool-approval-tool-call";
import { ToolApprovalPolicySettings } from "@/components/organizations/tool-approval-policy-settings";
import type { Organization, ToolCompletedData, ToolReviewedData } from "@/lib/api/types";
import { capApprovalMode, effectiveApprovalMode, switchableModes } from "@/lib/tool-approval";

const mockOrganization: { current: Partial<Organization> | undefined } = { current: undefined };
const mutateAsync = jest.fn();

jest.mock("@/hooks/use-organizations", () => ({
  useOrganization: () => ({ data: mockOrganization.current }),
  useUpdateOrganization: () => ({ mutateAsync, isPending: false }),
}));
jest.mock("@/lib/api/audit-logs", () => ({ listAuditLogs: jest.fn(async () => []) }));
jest.mock("@/lib/api/sessions", () => ({ submitToolApprovals: jest.fn() }));
jest.mock("@tanstack/react-query", () => ({
  useQuery: () => ({ data: [], isLoading: false, isError: false }),
}));

beforeEach(() => {
  mockOrganization.current = undefined;
  mutateAsync.mockReset();
});

describe("approval mode resolution", () => {
  it("raises a disallowed mode to the next more careful one", () => {
    expect(capApprovalMode("off", ["normal", "protective"])).toBe("normal");
    expect(capApprovalMode("auto", [])).toBe("auto");
    expect(capApprovalMode("normal", ["auto"])).toBe("protective");
  });

  it("runs a switch to Off as Auto when the agent's manager turned approvals on", () => {
    expect(effectiveApprovalMode("off", "normal", [])).toBe("auto");
    expect(effectiveApprovalMode("off", "off", [])).toBe("off");
    expect(effectiveApprovalMode(null, "protective", [])).toBe("protective");
    expect(effectiveApprovalMode("auto", "normal", ["normal", "protective"])).toBe("normal");
  });

  it("lists only modes the organization allows", () => {
    expect(switchableModes([])).toEqual(["auto", "normal", "protective", "off"]);
    expect(switchableModes(["normal", "protective"])).toEqual(["normal", "protective"]);
  });
});

describe("ToolApprovalConfigEditor", () => {
  it("offers every mode and writes the picked one, keeping fields it does not own", () => {
    const onChange = jest.fn();
    render(
      <ToolApprovalConfigEditor
        config={{ mode: "normal", timeout_seconds: 600 }}
        onChange={onChange}
      />,
    );
    fireEvent.click(screen.getByLabelText(/^Auto/));
    expect(onChange).toHaveBeenLastCalledWith({ mode: "auto", timeout_seconds: 600 });
  });

  it("hides Auto when the organization does not allow it", () => {
    mockOrganization.current = {
      tool_approval_policy: { allowed_modes: ["normal", "protective"] },
    };
    render(<ToolApprovalConfigEditor config={{ mode: "normal" }} onChange={jest.fn()} />);
    expect(screen.queryByLabelText(/^Auto/)).not.toBeInTheDocument();
    expect(
      screen.getByText("Auto shows only when the organization allows it."),
    ).toBeInTheDocument();
  });

  it("saves only filled-in rules and reviewer notes", () => {
    const onChange = jest.fn();
    function Controlled() {
      const [config, setConfig] = useState<Record<string, unknown>>({ mode: "auto" });
      return (
        <ToolApprovalConfigEditor
          config={config}
          onChange={(next) => {
            onChange(next);
            setConfig(next);
          }}
        />
      );
    }
    render(<Controlled />);
    fireEvent.click(screen.getByRole("button", { name: "Add rule" }));
    expect(onChange).toHaveBeenLastCalledWith({ mode: "auto" });
    fireEvent.change(screen.getByLabelText("Tool name or pattern"), {
      target: { value: " github_merge_* " },
    });
    expect(onChange).toHaveBeenLastCalledWith({
      mode: "auto",
      rules: [{ tool: "github_merge_*", action: "ask" }],
    });
    fireEvent.click(screen.getByRole("button", { name: "Add a note" }));
    fireEvent.change(screen.getByLabelText("Note 1"), { target: { value: "acme.com is ours." } });
    expect(onChange).toHaveBeenLastCalledWith({
      mode: "auto",
      rules: [{ tool: "github_merge_*", action: "ask" }],
      auto: { notes: ["acme.com is ours."] },
    });
  });
});

describe("ToolApprovalPolicySettings", () => {
  const organization = { id: "org_1", name: "Acme" } as Organization;

  it("saves an empty allowed list when every mode stays allowed", async () => {
    mutateAsync.mockResolvedValue({ ...organization, tool_approval_policy: {} });
    render(<ToolApprovalPolicySettings organization={organization} canManage />);
    fireEvent.click(screen.getByRole("button", { name: "Add a note" }));
    fireEvent.change(screen.getByLabelText("Note 1"), { target: { value: "Prod is prod-*." } });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() =>
      expect(mutateAsync).toHaveBeenCalledWith({
        tool_approval_policy: { allowed_modes: [], rules: [], trust_notes: ["Prod is prod-*."] },
      }),
    );
  });

  it("names the allowed modes once one is turned off", async () => {
    mutateAsync.mockResolvedValue(organization);
    render(<ToolApprovalPolicySettings organization={organization} canManage />);
    fireEvent.click(document.getElementById("org-approval-mode-off")!);
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() =>
      expect(mutateAsync).toHaveBeenCalledWith({
        tool_approval_policy: {
          allowed_modes: ["auto", "normal", "protective"],
          rules: [],
          trust_notes: [],
        },
      }),
    );
  });

  it("refuses to save with no mode allowed", async () => {
    render(<ToolApprovalPolicySettings organization={organization} canManage />);
    for (const mode of ["off", "auto", "normal", "protective"]) {
      fireEvent.click(document.getElementById(`org-approval-mode-${mode}`)!);
    }
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("Allow at least one mode.");
    expect(mutateAsync).not.toHaveBeenCalled();
  });

  it("is read-only for people who cannot manage the organization", () => {
    render(<ToolApprovalPolicySettings organization={organization} canManage={false} />);
    expect(screen.queryByRole("button", { name: "Save" })).not.toBeInTheDocument();
    expect(
      screen.getByText("Only organization owners and admins can change these settings."),
    ).toBeInTheDocument();
  });
});

function review(verdict: string, extra: Partial<ToolReviewedData> = {}): ToolReviewedData {
  return {
    tool_call_id: "call_1",
    tool_name: "bash",
    verdict,
    stage: "careful",
    reason: "Uploads a credentials file to a third-party site.",
    fingerprint: "sha256:ab",
    ...extra,
  };
}

describe("review marks", () => {
  it("keeps an allowed call quiet, with the reason on hover", () => {
    render(<ToolReviewBadge review={review("allowed", { reason: "Routine git move." })} />);
    expect(screen.getByTestId("tool-review-allowed")).toHaveAttribute("title", "Routine git move.");
  });

  it("shows a block's rule and reason in full", () => {
    const blocked = review("blocked", { rule: "Data Exfiltration" });
    render(
      <>
        <ToolReviewBadge review={blocked} />
        <ToolReviewBlockReason review={blocked} />
      </>,
    );
    expect(screen.getByText("Blocked")).toBeInTheDocument();
    expect(screen.getByTestId("tool-review-block-reason")).toHaveTextContent(
      "Data Exfiltration: Uploads a credentials file to a third-party site.",
    );
  });

  it("shows nothing for a call handed to a person", () => {
    const { container } = render(<ToolReviewBadge review={review("asked_person")} />);
    expect(container).toBeEmptyDOMElement();
  });
});

describe("approval card from the auto reviewer", () => {
  it("says why the reviewer asked", () => {
    render(
      <ToolApprovalRequests
        sessionId="session_1"
        toolResultsMap={new Map<string, ToolCompletedData>()}
        requests={[
          {
            id: "tool_approval_call_1",
            arguments: {
              tool: "github_merge_pull_request",
              tool_call_id: "call_1",
              arguments: { pull: 412 },
              risk: "destructive",
              review_note: "Could not tell whether you wanted PR #412 merged or only opened.",
            },
          },
        ]}
      />,
    );
    expect(screen.getByTestId("approval-review-note")).toHaveTextContent(
      "Why you're asked: Could not tell whether you wanted PR #412 merged or only opened.",
    );
  });

  it("says when auto review paused after too many blocks", () => {
    const paused = new Map([
      ["call_9", review("asked_person", { tool_call_id: "call_9", stage: "paused" })],
    ]);
    render(
      <ToolReviewsContext.Provider value={paused}>
        <ToolApprovalRequests
          sessionId="session_1"
          toolResultsMap={new Map<string, ToolCompletedData>()}
          requests={[
            {
              id: "tool_approval_call_9",
              arguments: { tool: "bash", tool_call_id: "call_9", arguments: { command: "ls" } },
            },
          ]}
        />
      </ToolReviewsContext.Provider>,
    );
    expect(screen.getByTestId("approval-review-paused")).toHaveTextContent(/paused/);
  });
});
