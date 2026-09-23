"use client";

// Project context and provider — the active project within the current org.
// Mirrors OrgProvider: selection persisted in localStorage, synced to a
// server-side cookie (everruns_project) so SSE requests are scoped too. On a
// project switch we invalidate all project-scoped queries and redirect entity
// detail pages to their list to avoid cross-project 404s.

import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { usePathname, useRouter } from "next/navigation";
import { useQueryClient } from "@tanstack/react-query";
import { switchProject as switchProjectApi } from "@/lib/api/users";
import { useProjects } from "@/hooks/use-projects";
import { useOrg } from "@/providers/org-provider";
import { PROJECT_STORAGE_KEY } from "@/lib/constants";
import type { Project } from "@/lib/api/types";

// Entity detail route prefixes — when on a detail page under one of these, a
// project switch redirects to the list page to avoid a 404. Kept in sync with
// the project-scoped dynamic [id] routes under apps/ui/src/app/(main)/.
const ENTITY_PREFIXES = [
  "/agents/",
  "/apps/",
  "/capabilities/",
  "/mcp-servers/",
  "/memory/",
  "/sessions/",
  "/skills/",
];

function getEntityListPath(pathname: string): string | null {
  for (const prefix of ENTITY_PREFIXES) {
    if (pathname.startsWith(prefix)) return prefix.slice(0, -1);
  }
  return null;
}

export interface ProjectContextValue {
  /** Currently selected project (within the active org). */
  currentProject: Project | null;
  /** All projects in the active org. */
  projects: Project[];
  /** Switch the active project (awaits cookie sync before committing). */
  setCurrentProject: (project: Project) => void;
  isLoading: boolean;
  isSwitching: boolean;
}

const ProjectContext = createContext<ProjectContextValue | undefined>(undefined);

interface ProjectProviderProps {
  children: ReactNode;
  initialProjectId?: string | null;
}

export function ProjectProvider({ children, initialProjectId = null }: ProjectProviderProps) {
  const { currentOrg } = useOrg();
  const { data: projects = [], isLoading: projectsLoading } = useProjects();
  const [currentProject, setCurrentProjectState] = useState<Project | null>(null);
  const [isSwitching, setIsSwitching] = useState(false);
  const router = useRouter();
  const pathname = usePathname();
  const queryClient = useQueryClient();

  // The project list belongs to the active org. When the org changes we must
  // re-resolve the project from the new org's list (the old project is invalid).
  const orgId = currentOrg?.public_id ?? null;

  const pickDefault = useCallback(
    (list: Project[]): Project | null => list.find((p) => p.is_default) ?? list[0] ?? null,
    [],
  );

  // Resolve the current project whenever the list (or org) changes.
  useEffect(() => {
    if (projectsLoading || projects.length === 0) return;

    // Keep the current selection if it still exists in this org's list.
    if (currentProject && projects.some((p) => p.id === currentProject.id)) return;

    const storedId =
      initialProjectId ??
      (typeof window !== "undefined" ? localStorage.getItem(PROJECT_STORAGE_KEY) : null);
    const preferred = storedId ? projects.find((p) => p.id === storedId) : undefined;
    const next = preferred ?? pickDefault(projects);
    if (next) {
      setCurrentProjectState(next);
      // Keep the server cookie in sync for non-explicit changes (e.g. org switch).
      void switchProjectApi(next.id).catch(() => {});
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [projects, projectsLoading, orgId]);

  const switchCounterRef = useRef(0);

  const setCurrentProject = useCallback(
    (project: Project) => {
      if (currentProject?.id === project.id) return;
      const switchId = ++switchCounterRef.current;
      setIsSwitching(true);

      switchProjectApi(project.id)
        .then(() => {
          if (switchCounterRef.current !== switchId) return;
          setCurrentProjectState(project);
          if (typeof window !== "undefined") {
            localStorage.setItem(PROJECT_STORAGE_KEY, project.id);
          }
          // Project-scoped lists must refetch under the new scope.
          void queryClient.invalidateQueries();
          const listPath = getEntityListPath(pathname);
          if (listPath) router.push(listPath);
        })
        .catch((error) => {
          if (switchCounterRef.current !== switchId) return;
          console.error("Project switch failed, staying on current project:", error);
        })
        .finally(() => {
          if (switchCounterRef.current === switchId) setIsSwitching(false);
        });
    },
    [currentProject?.id, pathname, queryClient, router],
  );

  const value = useMemo<ProjectContextValue>(
    () => ({
      currentProject,
      projects,
      setCurrentProject,
      isLoading: projectsLoading,
      isSwitching,
    }),
    [currentProject, projects, setCurrentProject, projectsLoading, isSwitching],
  );

  return <ProjectContext.Provider value={value}>{children}</ProjectContext.Provider>;
}

export function useProject() {
  const context = useContext(ProjectContext);
  if (context === undefined) {
    throw new Error("useProject must be used within a ProjectProvider");
  }
  return context;
}
