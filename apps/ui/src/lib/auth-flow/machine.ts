// Executable model of the authentication flow's reachability contract.
//
// This is NOT a re-implementation of auth — it is a hand-maintained map of the
// states a person can reach across login / signup / OAuth / password-reset /
// email-verification, and the affordances each state surfaces. Its purpose is
// one invariant: from any starting situation, a user can always reach their
// goal via an affordance that actually WORKS for their account.
//
// The subtlety this model exists to capture: every auth screen already links
// somewhere ("Back to sign in" is everywhere), so a naive "does this state have
// an outgoing edge?" check passes trivially and proves nothing. A real dead end
// here is semantic — a screen offers a way forward that no-ops or misleads for a
// particular hidden account state. So edges carry a `worksFor` guard, and the
// solver only traverses edges that genuinely advance that account state's goal.
//
// Sourced from apps/ui/src/app/(auth)/* and crates/server/src/auth/routes.rs on
// main. When you change an auth page's affordances, update this model and keep
// machine.test.ts green — that is the guard against reopening a closed trap.

/** The hidden DB truth the user never sees — the variable everything turns on. */
export type AccountState =
  | "none" // S0 · no row for the email
  | "local_unverified" // S1 · password_hash set, email_verified = false
  | "local_verified" // S2 · password_hash set, email_verified = true
  | "oauth_only"; // S3 · no password_hash, provider = google/github

export const ACCOUNT_STATES: AccountState[] = [
  "none",
  "local_unverified",
  "local_verified",
  "oauth_only",
];

/** A concrete situation a user can be in while pursuing a goal. */
export type Screen =
  | "login.email"
  | "login.password"
  | "signup.form"
  | "signup.check_email"
  | "forgot.form"
  | "forgot.sent"
  | "reset.form"
  | "reset.invalid"
  | "verify.pending"
  | "verify.failed_with_email"
  | "verify.failed_no_email"
  | "oauth.rejected_permanent" // O3/O4 · link refused, permanent
  | "app.gated_on_verify"; // e.g. accepting an org invite while unverified

/** Terminal success states — reaching one means the goal is met. */
export type Goal = "authenticated" | "email_verified";

export type Node = Screen | Goal;

export const GOALS: Goal[] = ["authenticated", "email_verified"];

/**
 * A directed edge the UI surfaces. `worksFor` restricts it to the account
 * states for which the affordance actually advances the user — omit it and the
 * edge works for every account state. An edge that is *surfaced* but no-ops for
 * some account state must exclude that state from `worksFor` (that omission is
 * exactly how a trap is encoded).
 */
export interface Edge {
  from: Screen;
  /** The button/link label a user sees. */
  affordance: string;
  to: Node;
  worksFor?: AccountState[];
}

const ALL = ACCOUNT_STATES;

// The transition table. Read each `to` as "where a working affordance leads".
export const EDGES: Edge[] = [
  // --- Login door ---
  { from: "login.email", affordance: "Continue with email", to: "login.password", worksFor: ALL },
  // OAuth from the door works only for accounts that HAVE an oauth identity (or
  // none yet — first-time signup). A local password account cannot pivot here.
  { from: "login.email", affordance: "Continue with Google", to: "authenticated", worksFor: ["none", "oauth_only"] },
  // Password submit succeeds only for accounts that have a usable password.
  { from: "login.password", affordance: "Continue (submit password)", to: "authenticated", worksFor: ["local_unverified", "local_verified"] },
  // The credential-failure alert offers reset. It only advances a real password
  // user; for oauth_only it reaches forgot.sent but no email is ever produced,
  // so this edge is NOT valid for oauth_only (that omission is trap #1).
  { from: "login.password", affordance: "reset your password", to: "reset.form", worksFor: ["local_unverified", "local_verified"] },
  { from: "login.password", affordance: "Create an account", to: "signup.form", worksFor: ALL },
  { from: "login.password", affordance: "Back", to: "login.email", worksFor: ALL },

  // --- Signup ---
  { from: "signup.form", affordance: "Create account (new email)", to: "authenticated", worksFor: ["none"] },
  // Existing account → generic "check your email"; the emailed guidance is "log
  // in", so the working continuation is the login door.
  { from: "signup.form", affordance: "Create account (existing email)", to: "signup.check_email", worksFor: ["local_unverified", "local_verified", "oauth_only"] },
  { from: "signup.check_email", affordance: "Log in", to: "login.email", worksFor: ALL },
  { from: "signup.check_email", affordance: "Use a different email", to: "signup.form", worksFor: ALL },

  // --- Password reset ---
  { from: "reset.form", affordance: "Set new password", to: "authenticated", worksFor: ["local_unverified", "local_verified"] },
  { from: "reset.form", affordance: "Expired → request new", to: "reset.invalid", worksFor: ALL },
  { from: "reset.invalid", affordance: "Request a new link", to: "forgot.form", worksFor: ALL },
  { from: "forgot.form", affordance: "Send reset link", to: "forgot.sent", worksFor: ALL },
  // The reset email only fires for a local password user (is_local_password_user).
  // For oauth_only NO email is sent, so forgot.sent has no working continuation.
  { from: "forgot.sent", affordance: "emailed reset link", to: "reset.form", worksFor: ["local_unverified", "local_verified"] },
  { from: "forgot.sent", affordance: "Back to sign in", to: "login.email", worksFor: ALL },

  // --- Email verification ---
  { from: "verify.pending", affordance: "Resend verification email", to: "email_verified", worksFor: ["local_unverified"] },
  { from: "verify.failed_with_email", affordance: "Resend link", to: "email_verified", worksFor: ["local_unverified"] },
  // failed_no_email: copy says "Sign in to request a new verification email",
  // but no in-app screen issues one. Its only real edge is back to the door,
  // which does NOT lead to verification for an already-signed-in unverified
  // user — hence no worksFor entry reaching email_verified (trap #2).
  { from: "verify.failed_no_email", affordance: "Back to sign in", to: "login.email", worksFor: ALL },
  // The invite gate demands a verified email but surfaces no way to verify.
  { from: "app.gated_on_verify", affordance: "(none surfaced)", to: "app.gated_on_verify", worksFor: [] },

  // --- OAuth permanent rejection ---
  // Lands on /login?error=… with "try again" copy. The account is bound to a
  // different provider (or an unverified local twin); retrying never resolves
  // it, and password login is the real path only for accounts that have a
  // password. For oauth_only-bound-elsewhere there is no working edge.
  { from: "oauth.rejected_permanent", affordance: "Try again / continue with email", to: "login.email", worksFor: ALL },
];

/** Canonical entry screen for each (goal, account) scenario a user starts from. */
export interface Scenario {
  goal: Goal;
  account: AccountState;
  start: Screen;
  /** Human description of the situation, for test output. */
  situation: string;
}

export const SCENARIOS: Scenario[] = [
  { goal: "authenticated", account: "local_verified", start: "login.email", situation: "Verified user signs in" },
  { goal: "authenticated", account: "local_verified", start: "forgot.form", situation: "Verified user who forgot their password recovers" },
  { goal: "authenticated", account: "none", start: "signup.form", situation: "Brand-new user creates an account" },
  { goal: "authenticated", account: "oauth_only", start: "login.password", situation: "Google user who typed a password instead recovers access" },
  { goal: "authenticated", account: "local_unverified", start: "login.email", situation: "Unverified user signs in with their password" },
  { goal: "email_verified", account: "local_unverified", start: "verify.pending", situation: "Unverified user verifies from the post-signup nudge" },
  { goal: "email_verified", account: "local_unverified", start: "app.gated_on_verify", situation: "Signed-in unverified user hits the invite gate and must verify" },
  { goal: "email_verified", account: "local_unverified", start: "verify.failed_no_email", situation: "Unverified user lands on an expired link with no email param" },
];

/** BFS over edges that WORK for the given account state. */
export function goalReachable(scenario: Scenario): boolean {
  const { goal, account, start } = scenario;
  const seen = new Set<Node>([start]);
  const queue: Node[] = [start];
  while (queue.length) {
    const node = queue.shift() as Node;
    if (node === goal) return true;
    for (const e of EDGES) {
      if (e.from !== node) continue;
      if (e.worksFor && !e.worksFor.includes(account)) continue;
      if (!seen.has(e.to)) {
        seen.add(e.to);
        queue.push(e.to);
      }
    }
  }
  return false;
}

/** The scenarios whose goal is currently unreachable — the structural dead ends. */
export function findDeadEnds(): Scenario[] {
  return SCENARIOS.filter((s) => !goalReachable(s));
}

/**
 * Semantic traps: a screen that surfaces an affordance *presented as the way
 * forward* which silently no-ops or misleads for a reachable account state,
 * without surfacing the working alternative on that same screen. These are NOT
 * caught by structural reachability — the working path often exists two hops
 * away (e.g. Back → Continue with Google), but the user, committed to the path
 * the screen offered, is never pointed at it. They require human judgement, so
 * they live as a reviewed registry rather than being derived.
 */
export interface Trap {
  screen: Screen;
  affordance: string;
  /** Account states for which the surfaced affordance is a no-op / dead wrong. */
  brokenFor: AccountState[];
  goal: Goal;
  note: string;
  /** Where in the source the broken behaviour lives. */
  loc: string;
}

export const TRAPS: Trap[] = [
  {
    screen: "login.password",
    affordance: "reset your password",
    brokenFor: ["oauth_only"],
    goal: "authenticated",
    note:
      "OAuth-only account: reset silently no-ops (is_local_password_user is false), and the screen never points to 'Continue with Google'. Enumeration-safe fix: generic copy naming OAuth as an alternative, shown to everyone.",
    loc: "login/page.tsx credential-failure alert · routes.rs forgot_password (1591)",
  },
  {
    screen: "oauth.rejected_permanent",
    affordance: "Try again / continue with email",
    brokenFor: ["local_unverified", "oauth_only"],
    goal: "authenticated",
    note:
      "Permanent link-refusal (email bound to another provider, or an unverified local twin) rendered as the transient 'didn't complete, try again' category because it raises 401 not 403. Retrying never resolves it; the working path is never named.",
    loc: "routes.rs oauth_callback (1184) · existing_oauth_link_rejection_reason (130)",
  },
];

/** All traps still open. Empty is the goal. */
export function findTraps(): Trap[] {
  return TRAPS;
}
