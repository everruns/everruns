import { render, screen } from "@testing-library/react";
import {
  SessionRepositories,
  findPullRequest,
  repositoryState,
} from "@/components/session/session-repositories";
import type { SessionRepository } from "@/lib/api/sandboxes";

function repository(patch: Partial<SessionRepository>): SessionRepository {
  return {
    repo: "acme/lib",
    path: "/workspace/lib",
    access: "write",
    branch: "everruns/bdaf0ce2",
    on_work_branch: true,
    base_branch: "main",
    uncommitted_files: 0,
    commits_ahead_of_base: 0,
    commits_not_pushed: 0,
    files: [],
    diff: "",
    diff_truncated: false,
    ...patch,
  } as SessionRepository;
}

const idle = {
  toolResultTexts: [],
  loading: false,
  refreshing: false,
  onRefresh: () => {},
};

describe("session repositories", () => {
  it("finds the newest pull request a tool reported for the repository", () => {
    const texts = [
      "opened https://github.com/acme/lib/pull/3",
      "other https://github.com/acme/app/pull/9",
      "updated https://github.com/acme/lib/pull/14 now",
    ];
    expect(findPullRequest("acme/lib", texts)).toEqual({
      number: 14,
      url: "https://github.com/acme/lib/pull/14",
    });
    expect(findPullRequest("acme/docs", texts)).toBeUndefined();
  });

  it("tells pushed work from work still in the sandbox", () => {
    expect(repositoryState(repository({}))).toBe("unchanged");
    expect(repositoryState(repository({ commits_ahead_of_base: 2 }))).toBe("pushed");
    expect(repositoryState(repository({ commits_ahead_of_base: 1, commits_not_pushed: 1 }))).toBe(
      "pending",
    );
    expect(repositoryState(repository({ uncommitted_files: 1 }))).toBe("pending");
  });

  it("shows each repository with its changes and pull request", () => {
    render(
      <SessionRepositories
        {...idle}
        sandboxState="running"
        toolResultTexts={["https://github.com/acme/lib/pull/14"]}
        repositories={[
          repository({
            commits_ahead_of_base: 2,
            files: [{ path: "src/config.py", additions: 6, deletions: 6, committed: true }],
            diff: "diff --git a/src/config.py b/src/config.py\n-def parse_config():\n+def load_config():",
          }),
          repository({
            repo: "acme/app",
            path: "/workspace/app",
            commits_ahead_of_base: 1,
            commits_not_pushed: 1,
            uncommitted_files: 1,
            files: [{ path: "setup.cfg", additions: 1, deletions: 1, committed: false }],
          }),
          repository({
            repo: "acme/docs",
            path: "/workspace/docs",
            access: "read",
            branch: "main",
          }),
        ]}
      />,
    );

    expect(screen.getByText(/2 commits ahead of main · pushed/)).toBeInTheDocument();
    expect(screen.getByRole("link", { name: /View pull request #14/ })).toHaveAttribute(
      "href",
      "https://github.com/acme/lib/pull/14",
    );
    expect(screen.getByText("+def load_config():")).toBeInTheDocument();
    expect(
      screen.getByText(/1 commit ahead of main · 1 file not committed · not pushed/),
    ).toBeInTheDocument();
    expect(screen.getByText("not committed")).toBeInTheDocument();
    expect(
      screen.getByText(/no changes · read only, the agent cannot push here/),
    ).toBeInTheDocument();
  });

  it("says why there is nothing to show when the sandbox is not running", () => {
    render(<SessionRepositories {...idle} sandboxState="stopped" repositories={[]} />);
    expect(screen.getByText(/The sandbox is not running/)).toBeInTheDocument();
  });
});
