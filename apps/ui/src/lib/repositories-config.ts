// The `repositories` capability config as the UI reads and writes it. Mirrors
// `RepositoriesConfig` in crates/integrations/src/github/repositories.rs.
//
// Values equal to the server's defaults are dropped on write, so a stored
// config says only what its author chose and later default changes still reach
// it. Fields this UI has no control for (`acts_as`) pass through untouched.

export const REPOSITORIES_CAPABILITY_ID = "repositories";

export type RepositoryAccess = "read" | "write";
export type AddRepositories = "off" | "read" | "write";

export interface RepositoryEntry {
  repo: string;
  /** Branch to start from; the repository's default branch when unset. */
  ref?: string;
  access: RepositoryAccess;
}

export interface RepositoriesConfig {
  repositories: RepositoryEntry[];
  add_repositories: AddRepositories;
  branch_prefix: string;
  depth: number;
  commit_name: string;
  commit_email: string;
  /** Everything else the stored config carries, kept as it is. */
  rest: Record<string, unknown>;
}

export const REPOSITORIES_DEFAULTS = {
  add_repositories: "read" as AddRepositories,
  branch_prefix: "everruns/",
  depth: 200,
  commit_name: "Everruns Agent",
  commit_email: "agent@everruns.invalid",
};

// Same shape the server accepts: owner/repo, each part with at least one
// character that is not a dot.
const REPO_PATTERN =
  /^[A-Za-z0-9_.-]*[A-Za-z0-9_-][A-Za-z0-9_.-]*\/[A-Za-z0-9_.-]*[A-Za-z0-9_-][A-Za-z0-9_.-]*$/;

export function isValidRepositoryName(repo: string): boolean {
  return REPO_PATTERN.test(repo);
}

/** `https://github.com/owner/repo(.git)` and `owner/repo` both name a repository. */
export function normalizeRepositoryName(input: string): string {
  return input
    .trim()
    .replace(/^https?:\/\/github\.com\//i, "")
    .replace(/\.git$/i, "")
    .replace(/\/+$/, "");
}

function text(value: unknown, fallback: string): string {
  return typeof value === "string" ? value : fallback;
}

export function readRepositoriesConfig(config: unknown): RepositoriesConfig {
  const raw =
    config && typeof config === "object" && !Array.isArray(config)
      ? (config as Record<string, unknown>)
      : {};
  const {
    repositories,
    add_repositories: addRepositories,
    branch_prefix: branchPrefix,
    depth,
    commit_name: commitName,
    commit_email: commitEmail,
    ...rest
  } = raw;
  const entries: RepositoryEntry[] = [];
  if (Array.isArray(repositories)) {
    for (const item of repositories) {
      if (!item || typeof item !== "object") continue;
      const entry = item as Record<string, unknown>;
      if (typeof entry.repo !== "string") continue;
      entries.push({
        repo: entry.repo,
        ...(typeof entry.ref === "string" && entry.ref ? { ref: entry.ref } : {}),
        access: entry.access === "write" ? "write" : "read",
      });
    }
  }
  return {
    repositories: entries,
    add_repositories:
      addRepositories === "off" || addRepositories === "write"
        ? addRepositories
        : REPOSITORIES_DEFAULTS.add_repositories,
    branch_prefix: text(branchPrefix, REPOSITORIES_DEFAULTS.branch_prefix),
    depth: typeof depth === "number" ? depth : REPOSITORIES_DEFAULTS.depth,
    commit_name: text(commitName, REPOSITORIES_DEFAULTS.commit_name),
    commit_email: text(commitEmail, REPOSITORIES_DEFAULTS.commit_email),
    rest,
  };
}

export function writeRepositoriesConfig(config: RepositoriesConfig): Record<string, unknown> {
  const out: Record<string, unknown> = { ...config.rest };
  if (config.repositories.length > 0) {
    out.repositories = config.repositories.map((entry) => ({
      repo: entry.repo,
      ...(entry.ref?.trim() ? { ref: entry.ref.trim() } : {}),
      ...(entry.access === "write" ? { access: "write" } : {}),
    }));
  }
  if (config.add_repositories !== REPOSITORIES_DEFAULTS.add_repositories) {
    out.add_repositories = config.add_repositories;
  }
  if (config.branch_prefix !== REPOSITORIES_DEFAULTS.branch_prefix) {
    out.branch_prefix = config.branch_prefix;
  }
  if (config.depth !== REPOSITORIES_DEFAULTS.depth) out.depth = config.depth;
  if (config.commit_name !== REPOSITORIES_DEFAULTS.commit_name) {
    out.commit_name = config.commit_name;
  }
  if (config.commit_email !== REPOSITORIES_DEFAULTS.commit_email) {
    out.commit_email = config.commit_email;
  }
  return out;
}

/**
 * The most a session may be given for `repo`: what the agent lists it with, or
 * what the agent allows for repositories it does not list. `undefined` means
 * the repository cannot be part of a session at all.
 */
export function allowedAccess(
  agent: RepositoriesConfig,
  repo: string,
): RepositoryAccess | undefined {
  const listed = agent.repositories.find(
    (entry) => entry.repo.toLowerCase() === repo.toLowerCase(),
  );
  if (listed) return listed.access;
  return agent.add_repositories === "off" ? undefined : agent.add_repositories;
}

function sameSet(left: RepositoryEntry[], right: RepositoryEntry[]): boolean {
  if (left.length !== right.length) return false;
  return left.every(
    (entry, index) =>
      entry.repo === right[index].repo &&
      entry.access === right[index].access &&
      (entry.ref ?? "") === (right[index].ref ?? ""),
  );
}

/**
 * The session-level capability config for a session that works on `picked`
 * instead of the agent's usual repositories, or `undefined` when the pick is
 * the agent's own set and the session needs no config of its own. A session's
 * config replaces the agent's whole, so everything else is carried over.
 */
export function sessionRepositoriesConfig(
  agentConfig: unknown,
  picked: RepositoryEntry[],
): Record<string, unknown> | undefined {
  const agent = readRepositoriesConfig(agentConfig);
  if (sameSet(agent.repositories, picked)) return undefined;
  return writeRepositoriesConfig({ ...agent, repositories: picked });
}
