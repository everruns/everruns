// Project types — the grouping layer nested inside an organization.
// See specs/multitenancy.md (Projects) and the "Project structure" design (F).

/** A project within the active organization (API shape). */
export interface Project {
  /** External identifier (proj_<32-hex-chars>). */
  id: string;
  name: string;
  description?: string | null;
  /** Whether this is the org's default project. */
  is_default: boolean;
  created_at: string;
  updated_at: string;
}

/** Request to create a project. */
export interface CreateProjectRequest {
  name: string;
  description?: string;
}

/** Request to update a project. */
export interface UpdateProjectRequest {
  name?: string;
  description?: string;
}
