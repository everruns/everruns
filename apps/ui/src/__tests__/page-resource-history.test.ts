import { formResource, resourceWrite } from "@/components/chat-mode/page-resource-history";
const id = "01933b5a000070008000000000000001";
const routes = [
  ["agents", "agent"],
  ["harnesses", "harness"],
  ["skills", "skill"],
  ["capabilities/declarative", "capability"],
  ["virtual-users", "virtual_user"],
  ["observers", "observer"],
  ["evals", "eval"],
  ["models", "model"],
  ["providers", "provider"],
  ["mcp-servers", "mcp_server"],
  ["knowledge-bases", "knowledge_base"],
  ["knowledge-indexes", "knowledge_index"],
  ["knowledge", "knowledge"],
  ["memories", "memory"],
  ["plugins", "plugin"],
  ["plugin_marketplaces", "plugin_marketplace"],
  ["workspaces", "workspace"],
  ["durable/schedules", "schedule"],
  ["reports/saved", "saved_report"],
  ["sandbox-templates", "sandbox_template"],
  ["orgs", "organization"],
];
it.each(routes)("reviews the real %s configuration endpoint", (path, kind) => {
  expect(resourceWrite("PATCH", `/v1/${path}/${id}`)).toMatchObject({ kind, id });
  expect(formResource(kind, id)).toMatchObject({ kind, id });
  expect(resourceWrite("POST", `/v1/${path}/${id}`)).toBeUndefined();
  expect(resourceWrite("PATCH", `/v1/${path}/${id}/credentials`)).toBeUndefined();
});
it.each([
  [`agents/agent_${id}/channels/appchan_${id}`, "agent_channel", `appchan_${id}`],
  [`agents/agent_${id}/triggers/trg_${id}`, "agent_trigger", `trg_${id}`],
  [`agents/agent_${id}/scripts/scr_${id}`, "agent_script", `scr_${id}`],
  [`knowledge-bases/kb_${id}/entries/kbe_${id}`, "knowledge_entry", `kbe_${id}`],
  [`evals/eval_${id}/cases/evalcase_${id}`, "eval_case", `evalcase_${id}`],
  [`orgs/org_${id}/organization-connections/${id}`, "organization_account", id],
])("reviews nested resource %s without using its parent as the subject", (path, kind, subject) => {
  expect(resourceWrite("PATCH", `/v1/${path}`)).toMatchObject({ kind, id: subject });
  expect(formResource(kind, subject)).toMatchObject({ kind, id: subject });
});
it("distinguishes settings sharing an organization identifier", () => {
  const org = `org_${id}`;
  expect(resourceWrite("PUT", `/v1/orgs/${org}/egress-allowlist`)).toMatchObject({
    kind: "organization_network",
    id: org,
  });
  expect(resourceWrite("PUT", "/v1/models/decision-default", org)).toMatchObject({
    kind: "organization_decision",
    id: org,
  });
  expect(formResource("model_defaults", org, "decision_model")?.kind).toBe("organization_decision");
  expect(formResource("web_search_defaults", org)?.kind).toBe("organization");
  expect(formResource("profile", id)?.kind).toBe("user_profile");
});
it.each([
  "invitation",
  "queued_task",
  "personal_access_token",
  "user_mcp_server",
  "knowledge_indexer",
  "manual_dialog",
  "playground_chat",
  "organization_setup",
])("does not invent saved Undo for a %s creation/action", (kind) => {
  expect(formResource(kind)).toBeUndefined();
  expect(formResource(kind, "new")).toBeUndefined();
});
