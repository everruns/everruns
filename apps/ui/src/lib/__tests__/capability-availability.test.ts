import { capabilitySelectable, capabilityTurnedOff } from "@/lib/capability-availability";

const skills = {
  status: "available" as const,
  feature_flag: "skills",
};

describe("capability availability", () => {
  it("hides a capability whose rollout flag is off", () => {
    expect(capabilitySelectable(skills, { skills: false })).toBe(false);
    expect(capabilityTurnedOff(skills, { skills: false })).toBe(true);
  });

  it("keeps a capability whose rollout flag is on", () => {
    expect(capabilitySelectable(skills, { skills: true })).toBe(true);
    expect(capabilityTurnedOff(skills, { skills: true })).toBe(false);
  });

  it("leaves gated capabilities selectable until flags are known", () => {
    expect(capabilitySelectable(skills, null)).toBe(true);
    expect(capabilityTurnedOff(skills, null)).toBe(false);
  });

  it("leaves ungated available capabilities selectable", () => {
    expect(capabilitySelectable({ status: "available" }, { skills: false })).toBe(true);
  });
});
