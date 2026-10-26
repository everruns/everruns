/** Full organization details (from API) */
export interface Organization {
  small_model_id?: string | null;
  id: string;
  name: string;
  default_model_id: string | null;
  default_harness_id: string | null;
  base_harness_id: string | null;
  /** Default provider per service kind (EVE-569); empty object when none. */
  default_provider_per_service: Record<string, string>;
  /** Who answers guardrail jev checks and the Slack relevance check. */
  system_decisions: SystemDecisionsSource;
  created_at: string;
  updated_at: string;
  /**
   * When the org's creator finished or skipped the setup wizard. `null` means
   * onboarding is incomplete; the resume redirect sends the user to /setup.
   */
  onboarding_completed_at: string | null;
}

/** Request to update an organization */
export interface UpdateOrganizationRequest {
  small_model_id?: string | null;
  name?: string;
  default_model_id?: string | null;
  default_harness_id?: string | null;
  base_harness_id?: string | null;
  /** Default provider per service kind (EVE-569); replaces the whole map. */
  default_provider_per_service?: Record<string, string>;
  system_decisions?: SystemDecisionsSource;
}

export type SystemDecisionsSource = "deployment" | "organization";
