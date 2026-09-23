// Shared constants used across providers and hooks.
// Avoids circular imports between org-provider and use-auth.

/** localStorage key for persisted org selection */
export const ORG_STORAGE_KEY = "everruns_current_org";

/** localStorage key for persisted project selection (per org) */
export const PROJECT_STORAGE_KEY = "everruns_current_project";

/** Default project public ID (matches backend DEFAULT_PROJECT_PUBLIC_ID) */
export const DEFAULT_PROJECT_PUBLIC_ID = "proj_00000000000000000000000000000001";
