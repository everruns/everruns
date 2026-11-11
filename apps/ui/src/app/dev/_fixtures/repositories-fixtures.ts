// Repositories fixtures: one coding session that changed two repositories and
// read a third, plus the agent settings it started from.

import type { AgentGitHubStatus, GitHubRepository } from "@/lib/api/agent-github";
import type { SessionRepository } from "@/lib/api/sandboxes";

export const repositoriesAgentId = "agent_dev_coder";
export const repositoriesIdentityId = "identity_dev_coder";

export const repositoriesAgentConfig = {
  repositories: [
    { repo: "acme/coding-test-app", access: "write" },
    { repo: "acme/coding-test-lib", access: "write" },
    { repo: "acme/coding-test-docs", ref: "main" },
  ],
};

export const repositoriesGitHubStatus: AgentGitHubStatus = {
  identity_id: repositoriesIdentityId,
  connected: true,
  app_created: true,
  app_slug: "everruns",
  app_url: null,
  kind: "everruns_app",
  bot_login: "everruns[bot]",
  app_settings_url: null,
  account: "acme",
  repository_selection: "selected",
  missing_permissions: [],
  suggested_owners: [],
  everruns_app_installations: [],
};

export const repositoriesReachable: GitHubRepository[] = [
  "coding-test-app",
  "coding-test-lib",
  "coding-test-docs",
  "coding-test-tools",
  "coding-test-infra",
].map((name) => ({
  full_name: `acme/${name}`,
  private: true,
  html_url: `https://github.com/acme/${name}`,
}));

const base = {
  on_work_branch: true,
  base_branch: "main",
  diff_truncated: false,
};

export const sessionRepositoriesFixture: SessionRepository[] = [
  {
    ...base,
    repo: "acme/coding-test-lib",
    path: "/workspace/coding-test-lib",
    access: "write",
    branch: "everruns/bdaf0ce2",
    uncommitted_files: 0,
    commits_ahead_of_base: 2,
    commits_not_pushed: 0,
    files: [
      { path: "coding_test_lib/config.py", additions: 6, deletions: 6, committed: true },
      { path: "tests/test_config.py", additions: 1, deletions: 1, committed: true },
    ],
    diff: [
      "diff --git a/coding_test_lib/config.py b/coding_test_lib/config.py",
      "--- a/coding_test_lib/config.py",
      "+++ b/coding_test_lib/config.py",
      "@@ -12,7 +12,7 @@",
      "-def parse_config(path: Path) -> Config:",
      "+def load_config(path: Path) -> Config:",
      "     text = path.read_text()",
    ].join("\n"),
  },
  {
    ...base,
    repo: "acme/coding-test-app",
    path: "/workspace/coding-test-app",
    access: "write",
    branch: "everruns/bdaf0ce2",
    uncommitted_files: 1,
    commits_ahead_of_base: 1,
    commits_not_pushed: 1,
    files: [
      { path: "app/main.py", additions: 2, deletions: 2, committed: true },
      { path: "setup.cfg", additions: 1, deletions: 1, committed: false },
    ],
    diff: "",
  },
  {
    ...base,
    repo: "acme/coding-test-docs",
    path: "/workspace/coding-test-docs",
    access: "read",
    branch: "main",
    on_work_branch: false,
    uncommitted_files: 0,
    commits_ahead_of_base: 0,
    commits_not_pushed: 0,
    files: [],
    diff: "",
  },
] as SessionRepository[];
