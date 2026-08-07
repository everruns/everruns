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

// The machine spans three layers; every node is tagged so the model is honest
// about where a transition actually happens. The reachability check treats them
// uniformly, but the distinction matters for the spec and for knowing which
// side (UI vs server vs inbox) owns a given edge.
export type Layer =
  | "ui" // a screen the browser renders
  | "backend" // an API outcome the server decides (session vs confirmation, link-refusal)
  | "external"; // something outside our system — an email in an inbox, a token's TTL

/** A concrete situation a user (or the system) can be in while pursuing a goal. */
export type Screen =
  // -- ui --
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
  | "app.gated_on_verify" // e.g. accepting an org invite while unverified
  // -- backend outcomes --
  | "backend.confirmation_pending" // confirm-mode register: account made, NO session
  | "oauth.rejected_permanent" // O3/O4 · link refused (409 account-exists), permanent
  | "oauth.rejected_policy" // O2 · 403 not-permitted (domain / provider-unverified)
  // -- external (inbox / token) --
  | "email.verify_link" // a verification link sits in the inbox (24h TTL)
  | "email.reset_link"; // a reset link sits in the inbox (1h TTL)

/** Which layer each node belongs to. Guarded for completeness in the tests. */
export const NODE_LAYER: Record<Screen, Layer> = {
  "login.email": "ui",
  "login.password": "ui",
  "signup.form": "ui",
  "signup.check_email": "ui",
  "forgot.form": "ui",
  "forgot.sent": "ui",
  "reset.form": "ui",
  "reset.invalid": "ui",
  "verify.pending": "ui",
  "verify.failed_with_email": "ui",
  "verify.failed_no_email": "ui",
  "app.gated_on_verify": "ui",
  "backend.confirmation_pending": "backend",
  "oauth.rejected_permanent": "backend",
  "oauth.rejected_policy": "backend",
  "email.verify_link": "external",
  "email.reset_link": "external",
};

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
  {
    from: "login.email",
    affordance: "Continue with Google",
    to: "authenticated",
    worksFor: ["none", "oauth_only"],
  },
  // Password submit succeeds only for accounts that have a usable password.
  {
    from: "login.password",
    affordance: "Continue (submit password)",
    to: "authenticated",
    worksFor: ["local_unverified", "local_verified"],
  },
  // The credential-failure alert offers reset. It only advances a real password
  // user; for oauth_only it reaches forgot.sent but no email is ever produced,
  // so this edge is NOT valid for oauth_only (that omission is trap #1).
  {
    from: "login.password",
    affordance: "reset your password",
    to: "reset.form",
    worksFor: ["local_unverified", "local_verified"],
  },
  { from: "login.password", affordance: "Create an account", to: "signup.form", worksFor: ALL },
  { from: "login.password", affordance: "Back", to: "login.email", worksFor: ALL },

  // --- Signup (backend outcome depends on AUTH_SIGNUP_EMAIL_CONFIRM) ---
  // Instant mode (self-host, no email sender): new signup mints a session now.
  {
    from: "signup.form",
    affordance: "Create account (instant mode)",
    to: "authenticated",
    worksFor: ["none"],
  },
  // Confirm mode (dev/prod): the server returns NO session — a generic
  // ConfirmationSent — for BOTH a new and an existing email (anti-enumeration).
  {
    from: "signup.form",
    affordance: "Create account (confirm mode)",
    to: "backend.confirmation_pending",
    worksFor: ALL,
  },
  {
    from: "backend.confirmation_pending",
    affordance: "→ Check your email screen",
    to: "signup.check_email",
    worksFor: ALL,
  },
  // A genuinely new / unverified account gets a verification link in the inbox;
  // an existing verified account gets the "you already have an account" email
  // instead (no verify link) — its working path is "Log in".
  {
    from: "signup.check_email",
    affordance: "verification link emailed",
    to: "email.verify_link",
    worksFor: ["none", "local_unverified"],
  },
  { from: "signup.check_email", affordance: "Log in", to: "login.email", worksFor: ALL },
  {
    from: "signup.check_email",
    affordance: "Use a different email",
    to: "signup.form",
    worksFor: ALL,
  },

  // --- External: the verification link in the inbox (24h TTL, single-use) ---
  // Clicking a valid link verifies AND mints a session (confirm-mode sign-in).
  {
    from: "email.verify_link",
    affordance: "click link (valid)",
    to: "authenticated",
    worksFor: ["none", "local_unverified"],
  },
  {
    from: "email.verify_link",
    affordance: "click link (valid)",
    to: "email_verified",
    worksFor: ["local_unverified"],
  },
  // Expired/used link lands on the failed state, which itself resends (below).
  {
    from: "email.verify_link",
    affordance: "click link (expired)",
    to: "verify.failed_with_email",
    worksFor: ALL,
  },

  // --- Password reset ---
  {
    from: "reset.form",
    affordance: "Set new password",
    to: "authenticated",
    worksFor: ["local_unverified", "local_verified"],
  },
  { from: "reset.form", affordance: "Expired → request new", to: "reset.invalid", worksFor: ALL },
  { from: "reset.invalid", affordance: "Request a new link", to: "forgot.form", worksFor: ALL },
  { from: "forgot.form", affordance: "Send reset link", to: "forgot.sent", worksFor: ALL },
  // The reset email only fires for a local password user (is_local_password_user).
  // For oauth_only NO email is produced, so forgot.sent has no external link —
  // that omission is why the OAuth-only reset path was a trap (now the login
  // door names the OAuth alternative instead).
  {
    from: "forgot.sent",
    affordance: "reset link emailed",
    to: "email.reset_link",
    worksFor: ["local_unverified", "local_verified"],
  },
  { from: "forgot.sent", affordance: "Back to sign in", to: "login.email", worksFor: ALL },
  // --- External: the reset link in the inbox (1h TTL, single-use) ---
  {
    from: "email.reset_link",
    affordance: "click link (valid)",
    to: "reset.form",
    worksFor: ["local_unverified", "local_verified"],
  },
  {
    from: "email.reset_link",
    affordance: "click link (expired)",
    to: "reset.invalid",
    worksFor: ALL,
  },

  // --- Email verification (in-app resend paths) ---
  {
    from: "verify.pending",
    affordance: "the emailed link",
    to: "email.verify_link",
    worksFor: ["local_unverified"],
  },
  {
    from: "verify.pending",
    affordance: "Resend verification email",
    to: "email.verify_link",
    worksFor: ["local_unverified"],
  },
  {
    from: "verify.failed_with_email",
    affordance: "Resend link",
    to: "email.verify_link",
    worksFor: ["local_unverified"],
  },
  // failed_no_email now takes an email and resends a fresh link in place,
  // instead of dead-ending on "sign in to request a new verification email".
  {
    from: "verify.failed_no_email",
    affordance: "Enter email → resend",
    to: "email.verify_link",
    worksFor: ["local_unverified"],
  },
  {
    from: "verify.failed_no_email",
    affordance: "Back to sign in",
    to: "login.email",
    worksFor: ALL,
  },
  // The persistent verify-email banner surfaces a resend on every authenticated
  // screen, so a signed-in unverified user hitting the invite gate always has a
  // working path to verification.
  {
    from: "app.gated_on_verify",
    affordance: "Verify-email banner · resend",
    to: "email.verify_link",
    worksFor: ["local_unverified"],
  },

  // --- OAuth rejections (backend-decided categories) ---
  // Permanent (409 → oauth_account_exists): the verified email already has an
  // unsafe local twin or the provider identity conflicts with an existing
  // binding. Copy names the way through via the original method.
  {
    from: "oauth.rejected_permanent",
    affordance: "use your original method",
    to: "login.email",
    worksFor: ALL,
  },
  // Policy (403 → oauth_not_permitted): domain/allow-list or provider-unverified.
  {
    from: "oauth.rejected_policy",
    affordance: "try a different account / email",
    to: "login.email",
    worksFor: ALL,
  },
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
  {
    goal: "authenticated",
    account: "local_verified",
    start: "login.email",
    situation: "Verified user signs in",
  },
  {
    goal: "authenticated",
    account: "local_verified",
    start: "forgot.form",
    situation: "Verified user who forgot their password recovers",
  },
  {
    goal: "authenticated",
    account: "none",
    start: "signup.form",
    situation: "Brand-new user creates an account",
  },
  {
    goal: "authenticated",
    account: "oauth_only",
    start: "login.password",
    situation: "Google user who typed a password instead recovers access",
  },
  {
    goal: "authenticated",
    account: "local_unverified",
    start: "login.email",
    situation: "Unverified user signs in with their password",
  },
  {
    goal: "email_verified",
    account: "local_unverified",
    start: "verify.pending",
    situation: "Unverified user verifies from the post-signup nudge",
  },
  {
    goal: "email_verified",
    account: "local_unverified",
    start: "app.gated_on_verify",
    situation: "Signed-in unverified user hits the invite gate and must verify",
  },
  {
    goal: "email_verified",
    account: "local_unverified",
    start: "verify.failed_no_email",
    situation: "Unverified user lands on an expired link with no email param",
  },
  // Cross-layer: exercise the backend confirm-mode outcome and the external
  // email hop, not just the UI screens.
  {
    goal: "authenticated",
    account: "none",
    start: "signup.form",
    situation: "Confirm-mode signup: the emailed link verifies and signs in",
  },
  {
    goal: "authenticated",
    account: "local_unverified",
    start: "signup.check_email",
    situation: "Confirm-mode signup whose email never arrived recovers by logging in",
  },
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

// Empty: both original traps are closed.
// - login.password reset-for-oauth-only now surfaces the OAuth alternative
//   ("Signed up with Google or GitHub? Go back and use that instead"), shown to
//   everyone so it reveals nothing.
// - the permanent OAuth link-refusal now returns a 409 → oauth_account_exists
//   category with copy that names the way through, instead of the transient
//   "didn't complete, try again".
// New traps get added here (and the ratchet in machine.test.ts flags them).
export const TRAPS: Trap[] = [];

/** All traps still open. Empty is the goal. */
export function findTraps(): Trap[] {
  return TRAPS;
}
