import { safeConsentUrl } from "@/lib/api/health-issues";
import { fixGroups } from "@/components/health/health-fix-queue";
import type { HealthIssue } from "@/lib/api/types";

test("only Slack's own approval pages are followed", () => {
  expect(safeConsentUrl("https://slack.com/oauth/v2/authorize?client_id=1&state=x")).not.toBeNull();
  expect(safeConsentUrl("https://api.slack.com/apps/A0123/install-on-team")).not.toBeNull();
  for (const bad of [
    "http://slack.com/oauth/v2/authorize",
    "https://slack.com.evil.example/oauth/v2/authorize",
    "https://slack.com/other",
    "https://api.slack.com/apps/A0123/install-on-team?redirect=x",
    "https://api.slack.com/apps/../install-on-team",
    "https://user@slack.com/oauth/v2/authorize",
    "javascript:alert(1)",
    "",
    null,
  ])
    expect(safeConsentUrl(bad)).toBeNull();
});

test("issues that share an available fixer are grouped", () => {
  const fixer = (kind: string, unavailable: string | null = null) => ({
    kind,
    label: "Fix",
    steps: [],
    effect: "consent",
    undoable: false,
    needs_provider_admin: true,
    unavailable_reason: unavailable,
  });
  const issue = (id: string, fixers: ReturnType<typeof fixer>[]) =>
    ({ id, fixers }) as unknown as HealthIssue;
  const groups = fixGroups([
    issue("a", [fixer("slack.add_permissions")]),
    issue("b", [fixer("slack.add_permissions")]),
    issue("c", [fixer("slack.add_permissions", "Connect first")]),
    issue("d", [fixer("schedule.pause")]),
  ]);
  expect(groups).toHaveLength(1);
  expect(groups[0][0]).toBe("slack.add_permissions");
  expect(groups[0][1].map((i) => i.id)).toEqual(["a", "b"]);
});
