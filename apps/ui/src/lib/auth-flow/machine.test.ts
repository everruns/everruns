import {
  ACCOUNT_STATES,
  EDGES,
  findDeadEnds,
  findTraps,
  GOALS,
  goalReachable,
  NODE_LAYER,
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
// Both are empty: every dead end and trap the audit found has been closed. A
// NEW entry appearing here means a regression was introduced and consciously
// parked; the goal is to keep both empty.

/** Situations where the goal is structurally unreachable (no working path). */
const KNOWN_DEAD_ENDS: string[] = [];

/** Screens that surface a broken remediation without the working alternative. */
const KNOWN_TRAPS: string[] = [];

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

  it("GOAL: no situation is a dead end", () => {
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

  it("GOAL: no misleading remediations remain", () => {
    expect(findTraps()).toHaveLength(0);
  });
});

describe("auth flow · model integrity", () => {
  // Derived from the layer map so the two can't drift apart.
  const SCREENS = Object.keys(NODE_LAYER) as Screen[];

  it("every node has a declared layer (ui / backend / external)", () => {
    for (const [node, layer] of Object.entries(NODE_LAYER)) {
      expect({ node, layer }).toEqual({
        node,
        layer: expect.stringMatching(/^(ui|backend|external)$/),
      });
    }
  });

  it("every edge references declared nodes and account states", () => {
    const nodes = new Set<Node>([...SCREENS, ...GOALS]);
    for (const e of EDGES) {
      expect(nodes.has(e.from)).toBe(true);
      expect(nodes.has(e.to)).toBe(true);
      for (const a of e.worksFor ?? []) {
        expect(ACCOUNT_STATES).toContain(a);
      }
    }
  });

  it("the model spans all three layers (not just UI screens)", () => {
    const layers = new Set(Object.values(NODE_LAYER));
    expect(layers).toEqual(new Set(["ui", "backend", "external"]));
  });

  it("every screen is the source of at least one affordance (no orphan states)", () => {
    for (const screen of SCREENS) {
      const out = EDGES.filter((e) => e.from === screen);
      expect({ screen, hasEdge: out.length > 0 }).toEqual({ screen, hasEdge: true });
    }
  });
});
