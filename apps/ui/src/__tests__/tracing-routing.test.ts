import { effectiveDestinations } from "@/lib/api/tracing";
import { formResource, resourceWrite } from "@/components/chat-mode/page-resource-history";
jest.mock("@/lib/api/client", () => ({ api: {} }));
const org = "org_00000000000000000000000000000001";
const agent = "agent_01933b5a000070008000000000000001";
it.each([
  ["inherit", [], ["default"]],
  ["add", ["extra", "default"], ["default", "extra"]],
  ["replace", ["extra"], ["extra"]],
  ["off", [], []],
] as const)("resolves %s without duplicate exports", (mode, selected, expected) => {
  expect(effectiveDestinations({ mode, destination_ids: [...selected] }, ["default"])).toEqual(
    expected,
  );
});
it("binds routing review to the correct organization or agent", () => {
  expect(resourceWrite("PUT", "/v1/tracing/defaults", org)).toMatchObject({
    kind: "tracing_policy",
    id: org,
  });
  expect(resourceWrite("PUT", `/v1/agents/${agent}/tracing`, org)).toMatchObject({
    kind: "tracing_policy",
    id: agent,
  });
  expect(formResource("tracing_policy", agent)).toMatchObject({
    kind: "tracing_policy",
    id: agent,
  });
  expect(resourceWrite("POST", "/v1/tracing/destinations", org)).toBeUndefined();
  expect(
    resourceWrite(
      "POST",
      "/v1/tracing/destinations/00000000-0000-0000-0000-000000000001/test",
      org,
    ),
  ).toBeUndefined();
});
