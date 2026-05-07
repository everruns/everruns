export type PaymentRail = "mpp_tempo" | "x402_base";
export type PaymentOwnerType = "user" | "agent_identity" | "organization";
export type PaymentStatus = "active" | "disabled" | "pending" | "succeeded" | "failed" | "released";

export interface PaymentAccount {
  id: string;
  organization_id: string;
  owner_type: PaymentOwnerType;
  owner_id: string;
  rail: PaymentRail;
  label: string;
  public_address?: string | null;
  status: PaymentStatus;
  metadata: Record<string, unknown>;
  created_at: string;
  updated_at: string;
}

export interface CreatePaymentAccountRequest {
  owner_type: PaymentOwnerType;
  owner_id: string;
  rail: PaymentRail;
  label: string;
  public_address?: string | null;
  private_key?: string;
  metadata?: Record<string, unknown>;
}

export interface UpdatePaymentAccountRequest {
  label?: string;
  public_address?: string | null;
  private_key?: string;
  status?: "active" | "disabled";
  metadata?: Record<string, unknown>;
}

export interface PaymentPolicy {
  id: string;
  organization_id: string;
  payment_account_id: string;
  subject_type: "user" | "agent_identity" | "agent" | "app" | "session" | "org";
  subject_id: string;
  allowed_capabilities: string[];
  allowed_hosts: string[];
  rail_preference: PaymentRail[];
  max_amount_usd_per_request?: number | null;
  max_amount_usd_per_turn?: number | null;
  max_amount_usd_per_day?: number | null;
  require_approval_above_usd?: number | null;
  status: PaymentStatus;
  metadata: Record<string, unknown>;
  created_at: string;
  updated_at: string;
}

export interface CreatePaymentPolicyRequest {
  payment_account_id: string;
  subject_type: PaymentPolicy["subject_type"];
  subject_id: string;
  allowed_capabilities: string[];
  allowed_hosts: string[];
  rail_preference: PaymentRail[];
  max_amount_usd_per_request?: number | null;
  max_amount_usd_per_turn?: number | null;
  max_amount_usd_per_day?: number | null;
  require_approval_above_usd?: number | null;
  metadata?: Record<string, unknown>;
}

export interface UpdatePaymentPolicyRequest {
  allowed_capabilities?: string[];
  allowed_hosts?: string[];
  rail_preference?: PaymentRail[];
  max_amount_usd_per_request?: number | null;
  max_amount_usd_per_turn?: number | null;
  max_amount_usd_per_day?: number | null;
  require_approval_above_usd?: number | null;
  status?: "active" | "disabled";
  metadata?: Record<string, unknown>;
}

export interface PaymentAttempt {
  id: string;
  organization_id: string;
  payment_account_id?: string | null;
  session_id?: string | null;
  capability: string;
  operation: string;
  rail?: PaymentRail | null;
  amount_usd: number;
  currency: string;
  target_url: string;
  request_hash?: string | null;
  status: PaymentStatus;
  error_message?: string | null;
  receipt: Record<string, unknown>;
  created_at: string;
  updated_at: string;
}
