import {
  PageChangeReviewController,
  type ReviewTransport,
} from "@/components/chat-mode/page-change-review-controller";
import { resourceWrite, type PageResource } from "@/components/chat-mode/page-resource-history";

const resource: PageResource = {
  kind: "model",
  id: "model_01933b5a000070008000000000000001",
  label: "Model",
};
const endpoint = `/v1/models/${resource.id}`;
function setup(initialRevision = 1) {
  let org = "org_test",
    revision = initialRevision;
  const transport: ReviewTransport = {
    currentOrg: () => org,
    history: jest.fn(async () => [{ revision, action: "updated" }]),
    diff: jest.fn(async () => [
      { field: "display_name", from: "Before", to: "After" },
      { field: "enabled", from: false, to: true },
    ]),
    restore: jest.fn(async () => ++revision),
    refresh: jest.fn(async () => {}),
  };
  const manager = new PageChangeReviewController(org, transport);
  const form = { id: "form", resource, dirty: false, baseline: "saved-1" };
  manager.connect(form);
  return {
    manager,
    transport,
    form,
    setRevision: (value: number) => {
      revision = value;
    },
    setOrg: (value: string) => {
      org = value;
    },
  };
}

it("captures a dirty form's revision and blocks saving after another writer commits", async () => {
  const { manager, form, setRevision } = setup();
  await manager.poll();
  manager.update({ ...form, dirty: true });
  setRevision(2);
  await manager.poll();
  expect(manager.guard(form.id).conflict).toBe(true);
  await expect(manager.beforeWrite("PATCH", endpoint)).rejects.toThrow(/saved version changed/i);
});

it("uses the committed receipt rather than including a later writer's change in Undo", async () => {
  const { manager, form, transport, setRevision } = setup();
  await manager.poll();
  manager.update({ ...form, dirty: true });
  const write = await manager.beforeWrite("PATCH", endpoint);
  expect(JSON.parse(write!.headers!["Everruns-Expected-Change"])).toEqual({
    kind: "model",
    id: resource.id,
    revision: 1,
  });
  setRevision(3); // This browser saved revision 2; another writer already made 3.
  await write!.succeeded({ ...resource, revision: 2 });
  expect(transport.diff).toHaveBeenCalledWith(resource, 1, 2);
  expect(manager.receipts()[0]).toMatchObject({ revision: 2, stale: true });
  manager.update({ ...form, baseline: "saved-3" });
  await manager.undo(resource);
  expect(transport.restore).not.toHaveBeenCalled();
});

it("undoes one field at the receipt revision and preserves the other saved field", async () => {
  const { manager, form, transport, setRevision } = setup();
  await manager.poll();
  const write = await manager.beforeWrite("PATCH", endpoint);
  setRevision(2);
  await write!.succeeded({ ...resource, revision: 2 });
  manager.update({ ...form, baseline: "saved-2" });
  await manager.undo(resource, "display_name");
  expect(transport.restore).toHaveBeenCalledWith(resource, 1, 2, ["display_name"]);
  expect(manager.receipts()[0]).toMatchObject({ revision: 3, fields: [{ field: "enabled" }] });
  await manager.undo(resource);
  expect(transport.restore).toHaveBeenLastCalledWith(resource, 1, 3, ["enabled"]);
  expect(manager.receipts()).toHaveLength(0);
  expect(manager.undoneNotices()).toEqual([{ resource, count: 1 }]);
});

it("confirms a completed Undo all until shown or replaced by the next save", async () => {
  const { manager, form, setRevision } = setup();
  await manager.poll();
  let write = await manager.beforeWrite("PATCH", endpoint);
  setRevision(2);
  await write!.succeeded({ ...resource, revision: 2 });
  manager.update({ ...form, baseline: "saved-2" });
  await manager.undo(resource);
  expect(manager.receipts()).toHaveLength(0);
  expect(manager.undoneNotices()).toEqual([{ resource, count: 2 }]);

  write = await manager.beforeWrite("PATCH", endpoint);
  setRevision(4);
  await write!.succeeded({ ...resource, revision: 4 });
  expect(manager.receipts()).toHaveLength(1);
  expect(manager.undoneNotices()).toHaveLength(0);

  await manager.undo(resource);
  manager.clearUndone(resource);
  expect(manager.undoneNotices()).toHaveLength(0);
});

it("keeps a failed restore visible and refuses Undo while a local draft is dirty", async () => {
  const { manager, form, transport, setRevision } = setup();
  await manager.poll();
  const write = await manager.beforeWrite("PATCH", endpoint);
  setRevision(2);
  await write!.succeeded({ ...resource, revision: 2 });
  manager.update({ ...form, dirty: true });
  await manager.undo(resource);
  expect(transport.restore).not.toHaveBeenCalled();
  manager.update({ ...form, dirty: false });
  (transport.restore as jest.Mock).mockRejectedValueOnce(new Error("Permission changed"));
  await manager.undo(resource);
  expect(manager.receipts()[0]).toMatchObject({
    error: "Permission changed",
    fields: [{ field: "display_name" }, { field: "enabled" }],
  });
});

it("does not turn missing review data into a retryable save failure", async () => {
  const { manager, transport } = setup();
  await manager.poll();
  const write = await manager.beforeWrite("PATCH", endpoint);
  (transport.history as jest.Mock).mockRejectedValueOnce(new Error("offline"));
  await expect(write!.succeeded({ ...resource, revision: 2 })).resolves.toBeUndefined();
  expect(manager.errors()[0].error).toMatch(/Saved successfully/);
});

it("drops receipts and late responses on organization change/disposal", async () => {
  const { manager, setOrg, transport } = setup();
  await manager.poll();
  const write = await manager.beforeWrite("PATCH", endpoint);
  manager.dispose();
  setOrg("org_other");
  await write!.succeeded({ ...resource, revision: 2 });
  expect(manager.receipts()).toHaveLength(0);
  expect(transport.diff).not.toHaveBeenCalled();
});

it("does not infer a configuration write from actions, credentials or creates", () => {
  for (const [method, path] of [
    ["POST", endpoint],
    ["POST", "/v1/durable/workers/demo/drain"],
    ["PUT", "/v1/credential-bindings/01933b5a000070008000000000000001"],
    ["PATCH", `${endpoint}/credentials`],
    ["DELETE", endpoint],
  ]) {
    expect(resourceWrite(method, path)).toBeUndefined();
  }
});

it("recovers the review after a successful save without repeating the mutation", async () => {
  const { manager, transport, setRevision } = setup();
  await manager.poll();
  const write = await manager.beforeWrite("PATCH", endpoint);
  setRevision(2);
  (transport.diff as jest.Mock).mockRejectedValueOnce(new Error("history temporarily offline"));
  await write!.succeeded({ ...resource, revision: 2 });
  expect(manager.errors()[0].error).toMatch(/Saved successfully/);
  await manager.retry(resource);
  expect(manager.receipts()[0]).toMatchObject({ from: 1, revision: 2, stale: false });
  expect(manager.errors()).toEqual([]);
});

it("does not poll old receipts after leaving their pages and checks them before Undo", async () => {
  const { manager, transport, setRevision } = setup();
  await manager.poll();
  const write = await manager.beforeWrite("PATCH", endpoint);
  setRevision(2);
  await write!.succeeded({ ...resource, revision: 2 });
  const other = {
    kind: "harness",
    id: "harness_01933b5a000070008000000000000001",
    label: "Harness",
  };
  manager.activeResource = other;
  // Disconnect the original form by disposing/reconnecting would erase receipts;
  // this fixture instead uses an unmounted-resource write, as inline list toggles do.
  setRevision(1);
  const standalone = new PageChangeReviewController("org_test", transport);
  const standaloneWrite = await standalone.beforeWrite("PATCH", endpoint);
  setRevision(2);
  await standaloneWrite!.succeeded({ ...resource, revision: 2 });
  expect(standalone.receipts()).toHaveLength(1);
  standalone.activeResource = other;
  (transport.history as jest.Mock).mockClear();
  await standalone.poll();
  expect(transport.history).toHaveBeenCalledTimes(1);
  expect(transport.history).toHaveBeenCalledWith(other);
  setRevision(3);
  await standalone.undo(resource);
  expect(transport.restore).not.toHaveBeenCalled();
});

it("reconnects after Strict Mode cleanup without accepting a late previous-epoch read", async () => {
  const { manager, transport, form } = setup();
  await manager.poll();
  let old!: (value: { revision: number; action: string }[]) => void;
  (transport.history as jest.Mock).mockImplementationOnce(
    () =>
      new Promise((resolve) => {
        old = resolve;
      }),
  );
  const pending = manager.poll();
  manager.dispose();
  manager.connect(form);
  await manager.poll();
  old([{ revision: 99, action: "updated" }]);
  await pending;
  expect(manager.guard(form.id)).toEqual({ loading: false, conflict: false, error: undefined });
  const write = await manager.beforeWrite("PATCH", endpoint);
  expect(JSON.parse(write!.headers!["Everruns-Expected-Change"]).revision).toBe(1);
  write!.failed();
});

it("retains a model selection after failed atomic Undo, then undoes only one chosen model", async () => {
  const { manager, transport } = setup();
  await manager.poll();
  const members = ["first", "second"].map((id) => ({
    id,
    label: id,
    from_revision: 1,
    revision: 2,
    before: false,
    after: true,
  }));
  manager.recordSelection(members);
  const receipt = manager.receipts()[0];
  transport.undoSelection = jest
    .fn()
    .mockRejectedValueOnce(new Error("A model changed"))
    .mockResolvedValueOnce([{ ...members[0], revision: 3, before: true, after: false }]);
  await manager.undo(receipt.resource);
  expect(receipt.fields).toHaveLength(2);
  expect(receipt.error).toBe("A model changed");
  await manager.undo(receipt.resource, "first");
  expect(transport.undoSelection).toHaveBeenLastCalledWith([
    { id: "first", from_revision: 1, expected_revision: 2 },
  ]);
  expect(receipt.fields).toEqual([{ field: "second", from: false, to: true }]);
});

it("bounds retained review groups without dropping the active page's state", async () => {
  const { manager } = setup();
  await manager.poll();
  for (let index = 0; index < 30; index++)
    manager.recordSelection([
      {
        id: `model-${index}`,
        label: `Model ${index}`,
        from_revision: 1,
        revision: 2,
        before: false,
        after: true,
      },
    ]);
  expect(manager.receipts()).toHaveLength(20);
  expect(manager.guard("form").loading).toBe(false);
});

it("does not include a foreign writer's save between two Chat polls in Undo", async () => {
  const { manager, transport } = setup();
  await manager.poll();
  manager.threadId = "current-chat";
  (transport.history as jest.Mock).mockResolvedValueOnce([
    { revision: 4, action: "updated", via_session_id: "current-chat" },
    { revision: 3, action: "updated", via_session_id: "current-chat" },
    { revision: 2, action: "updated", via_session_id: "another-writer" },
    { revision: 1, action: "created" },
  ]);
  await manager.poll();
  expect(transport.diff).toHaveBeenCalledWith(resource, 2, 4);
  expect(manager.receipts()[0]).toMatchObject({ from: 2, revision: 4 });
});

it("uses the server's captured baseline when first writing a migrated resource", async () => {
  const { manager, transport, setRevision } = setup();
  await manager.poll();
  const write = await manager.beforeWrite("PATCH", endpoint);
  setRevision(3);
  await write!.succeeded({ ...resource, revision: 3, from_revision: 2 });
  expect(transport.diff).toHaveBeenCalledWith(resource, 2, 3);
  expect(manager.receipts()[0]).toMatchObject({ from: 2, revision: 3 });
});

it.each([
  ["/v1/users/me", "user_profile"],
  ["/v1/virtual-users/me", "virtual_user"],
])("guards %s using only the explicitly registered form identity", async (path, kind) => {
  const { manager, transport } = setup();
  expect(await manager.beforeWrite("PATCH", path)).toBeUndefined();
  const own = { kind, id: "identity_01933b5a000070008000000000000002", label: "Profile" };
  manager.connect({ id: "own-profile", resource: own, dirty: false, baseline: "original" });
  await manager.poll();
  const write = await manager.beforeWrite("PATCH", path);
  expect(JSON.parse(write!.headers!["Everruns-Expected-Change"])).toEqual({
    kind,
    id: own.id,
    revision: 1,
  });
  await write!.succeeded({ ...own, revision: 2 });
  expect(transport.diff).toHaveBeenCalledWith(own, 1, 2);
});

it("bounds receipts from ordinary row saves without requiring page navigation", async () => {
  const { manager, setRevision } = setup();
  await manager.poll();
  for (let index = 1; index <= 30; index++) {
    const row = {
      kind: "model",
      id: `model_${index.toString(16).padStart(32, "0")}`,
      label: "Model",
    };
    setRevision(1);
    const write = await manager.beforeWrite("PATCH", `/v1/models/${row.id}`);
    setRevision(2);
    await write!.succeeded({ ...row, revision: 2 });
  }
  expect(manager.receipts()).toHaveLength(20);
  expect(manager.guard("form").loading).toBe(false);
});

it("reviews the first Chat save on a resource that previously had no journal", async () => {
  const { manager, transport } = setup(0);
  await manager.poll();
  manager.threadId = "current-chat";
  (transport.history as jest.Mock).mockResolvedValueOnce([
    { revision: 2, action: "updated", via_session_id: "current-chat" },
    { revision: 1, action: "baseline", via_session_id: "current-chat" },
  ]);
  await manager.poll();
  expect(transport.diff).toHaveBeenCalledWith(resource, 1, 2);
  expect(manager.receipts()[0]).toMatchObject({ from: 1, revision: 2 });
});

it("does not invent a destructive inverse for a first Chat creation", async () => {
  const { manager, transport } = setup(0);
  await manager.poll();
  manager.threadId = "current-chat";
  (transport.history as jest.Mock).mockResolvedValueOnce([
    { revision: 1, action: "created", via_session_id: "current-chat" },
  ]);
  await manager.poll();
  expect(transport.diff).not.toHaveBeenCalled();
  expect(manager.receipts()).toHaveLength(0);
});
