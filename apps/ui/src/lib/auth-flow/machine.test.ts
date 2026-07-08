import {
  ACCOUNT_STATES,
  EDGES,
  findDeadEnds,
  findTraps,
  goalReachable,
  SCENARIOS,
  type Node,
  type Screen,
} from "@/lib/auth-flow/machine";

// The reachability contract for the auth flow. Two invariants, each with a
// ratchet: the currently-open gaps are listed explicitly, so the suite is green
// today but fails the moment a NEW dead end or trap is introduced — and the
// `it.failing` guards below flip to failing (prompting removal) the moment a
// listed gap is actually closed. Close a gap => delete it from the model AND
// from the KNOWN_* list in the same change.

// --- Ratchet lists: the gaps we know about, pending fixes. ---

/** Situations where the goal is structurally unreachable (no working path). */
const KNOWN_DEAD_ENDS = [
  "Signed-in unverified user hits the invite gate and must verify",
  "Unverified user lands on an expired link with no email param",
];

/** Screens that surface a broken remediation without the working alternative. */
const KNOWN_TRAPS = ["login.password:reset your password", "oauth.rejected_permanent:Try again / continue with email"];

describe("auth flow · structural reachability", () => {
  it("every scenario's goal is reachable except the known dead ends", () => {
    const open = findDeadEnds()
      .map((s) => s.situation)
      .sort();
    expect(open).toEqual([...KNOWN_DEAD_ENDS].sort());
  });

  it("the healthy scenarios really are reachable (guards the model itself)", () => {
    const healthy = SCENARIOS.filter((s) => !KNOWN_DEAD_ENDS.includes(s.situation));
    for (const s of healthy) {
      expect({ situation: s.situation, reachable: goalReachable(s) }).toEqual({
        situation: s.situation,
        reachable: true,
      });
    }
  });

  it.failing("GOAL: no situation is a dead end (flip to it() once all are fixed)", () => {
    expect(findDeadEnds()).toHaveLength(0);
  });
});

describe("auth flow · misleading remediations", () => {
  it("the only open traps are the known ones", () => {
    const open = findTraps()
      .map((t) => `${t.screen}:${t.affordance}`)
      .sort();
    expect(open).toEqual([...KNOWN_TRAPS].sort());
  });

  it("every trap documents who it breaks for and where to fix it", () => {
    for (const t of findTraps()) {
      expect(t.brokenFor.length).toBeGreaterThan(0);
      expect(t.note.length).toBeGreaterThan(0);
      expect(t.loc.length).toBeGreaterThan(0);
    }
  });

  it.failing("GOAL: no misleading remediations remain (flip once all are fixed)", () => {
    expect(findTraps()).toHaveLength(0);
  });
});

describe("auth flow · model integrity", () => {
  const SCREENS: Screen[] = [
    "login.email",
    "login.password",
    "signup.form",
    "signup.check_email",
    "forgot.form",
    "forgot.sent",
    "reset.form",
    "reset.invalid",
    "verify.pending",
    "verify.failed_with_email",
    "verify.failed_no_email",
    "oauth.rejected_permanent",
    "app.gated_on_verify",
  ];

  it("every edge references declared nodes and account states", () => {
    const nodes = new Set<Node>([...SCREENS, "authenticated", "email_verified"]);
    for (const e of EDGES) {
      expect(nodes.has(e.from)).toBe(true);
      expect(nodes.has(e.to)).toBe(true);
      for (const a of e.worksFor ?? []) {
        expect(ACCOUNT_STATES).toContain(a);
      }
    }
  });

  it("every screen is the source of at least one affordance (no orphan states)", () => {
    for (const screen of SCREENS) {
      const out = EDGES.filter((e) => e.from === screen);
      expect({ screen, hasEdge: out.length > 0 }).toEqual({ screen, hasEdge: true });
    }
  });
});
