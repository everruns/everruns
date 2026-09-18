import {
  exposureStateLabel,
  isAnonymousExposure,
  resolveExposureState,
  type ExposureState,
} from "@/hooks/use-org-exposures";
import type { Agent, AppChannel } from "@/lib/api/types";

function channel(overrides: Partial<AppChannel> = {}): AppChannel {
  return {
    id: "appchan_test",
    channel_type: "webhook",
    channel_config: {},
    enabled: true,
    status: "live",
    created_at: "2026-05-10T00:00:00Z",
    updated_at: "2026-05-10T00:00:00Z",
    ...overrides,
  } as AppChannel;
}

function agent(overrides: Partial<Agent> = {}): Agent {
  return {
    id: "agent_test",
    status: "active",
    exposures_suspended: false,
    ...overrides,
  } as Agent;
}

// Pinned to `live(endpoint)` in crates/server/src/api/app_ingress.rs:
//
//   live = endpoint.status == live && agent.status == active
//                                  && !agent.exposures_suspended
//
// The view exists to be trusted during an incident, so it must never report
// something live that the server would refuse. Change these together.
describe("exposure state resolution", () => {
  const cases: Array<[string, AppChannel, Agent | undefined, ExposureState]> = [
    ["a live endpoint on an active agent", channel(), agent(), "live"],
    ["a draft endpoint", channel({ status: "draft" }), agent(), "draft"],
    ["a disabled endpoint", channel({ status: "disabled" }), agent(), "disabled"],
    ["an endpoint turned off via `enabled`", channel({ enabled: false }), agent(), "disabled"],
    [
      "a live endpoint whose agent is suspended",
      channel(),
      agent({ exposures_suspended: true }),
      "suspended",
    ],
    [
      "a live endpoint whose agent is archived",
      channel(),
      agent({ status: "archived" }),
      "agent-inactive",
    ],
    ["a live endpoint with no agent at all", channel(), undefined, "agent-inactive"],
  ];

  it.each(cases)("resolves %s", (_name, ch, ag, expected) => {
    expect(resolveExposureState(ch, ag)).toBe(expected);
  });

  // The agent-level terms win over the endpoint's own state, exactly as the
  // server folds them in at resolution time. A suspended agent's draft
  // endpoint is still not live, and must not read as merely "draft" — the
  // operator needs to see that the agent is the reason.
  it("reports the agent-level reason even when the endpoint is also not live", () => {
    expect(
      resolveExposureState(channel({ status: "draft" }), agent({ exposures_suspended: true })),
    ).toBe("suspended");
  });

  it("labels every state", () => {
    const states: ExposureState[] = ["live", "draft", "disabled", "suspended", "agent-inactive"];
    for (const state of states) {
      expect(exposureStateLabel(state)).toBeTruthy();
    }
  });
});

// Anonymous and publicly-reachable are different questions, and conflating
// them hid a real hazard: a suspended agent's anonymous endpoint read as
// "authenticated" until someone resumed the agent, at which point it was
// instantly open with no prior warning in the row.
describe("anonymous configuration versus live reachability", () => {
  const anonymousAgUi = channel({ channel_type: "ag_ui", channel_config: {} });

  it("an anonymous endpoint is anonymous even while suspended", () => {
    expect(isAnonymousExposure(anonymousAgUi)).toBe(true);
    expect(resolveExposureState(anonymousAgUi, agent({ exposures_suspended: true }))).toBe(
      "suspended",
    );
  });

  it("an anonymous endpoint is anonymous even while draft", () => {
    const draft = channel({ channel_type: "ag_ui", channel_config: {}, status: "draft" });
    expect(isAnonymousExposure(draft)).toBe(true);
    expect(resolveExposureState(draft, agent())).toBe("draft");
  });

  it("only an anonymous endpoint that is also live is an open door", () => {
    const openNow =
      isAnonymousExposure(anonymousAgUi) && resolveExposureState(anonymousAgUi, agent()) === "live";
    const suspended =
      isAnonymousExposure(anonymousAgUi) &&
      resolveExposureState(anonymousAgUi, agent({ exposures_suspended: true })) === "live";
    expect(openNow).toBe(true);
    expect(suspended).toBe(false);
  });
});

describe("publicly reachable detection", () => {
  it("counts an anonymous public chat endpoint", () => {
    expect(
      isAnonymousExposure(
        channel({ channel_type: "public_chat", channel_config: { anonymous: true } }),
      ),
    ).toBe(true);
  });

  it("counts an AG-UI endpoint with no token and no sign-in", () => {
    expect(isAnonymousExposure(channel({ channel_type: "ag_ui", channel_config: {} }))).toBe(true);
  });

  it("does not count one behind a token", () => {
    expect(
      isAnonymousExposure(
        channel({ channel_type: "ag_ui", channel_config: { token_configured: true } }),
      ),
    ).toBe(false);
  });

  it("prefers first-class auth over legacy nested auth", () => {
    expect(
      isAnonymousExposure(
        channel({
          channel_type: "public_chat",
          auth: { mode: "google_oidc" },
          channel_config: { anonymous: true, auth: { mode: "anonymous" } },
        }),
      ),
    ).toBe(false);
    expect(
      isAnonymousExposure(
        channel({
          channel_type: "public_chat",
          auth: { mode: "anonymous" },
          channel_config: { anonymous: true, auth: { mode: "google_oidc" } },
        }),
      ),
    ).toBe(true);
  });

  it("does not count one behind a sign-in provider", () => {
    expect(
      isAnonymousExposure(
        channel({
          channel_type: "public_chat",
          channel_config: { anonymous: true, auth: { mode: "google_oidc" } },
        }),
      ),
    ).toBe(false);
  });

  it("does not count one with anonymous access explicitly off", () => {
    expect(
      isAnonymousExposure(
        channel({ channel_type: "public_chat", channel_config: { anonymous: false } }),
      ),
    ).toBe(false);
  });

  // Every other transport authenticates by construction — Slack signs, webhook
  // and api_endpoint carry a token or key, A2A carries an API key, and a
  // schedule has no inbound caller at all.
  it.each(["slack", "webhook", "api_endpoint", "a2a", "fcp", "schedule"] as const)(
    "does not count %s as anonymous",
    (kind) => {
      expect(isAnonymousExposure(channel({ channel_type: kind, channel_config: {} }))).toBe(false);
    },
  );
});
