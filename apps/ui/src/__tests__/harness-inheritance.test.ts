import type { Harness } from "@/lib/api/types";
import { resolveHarnessInheritance } from "@/lib/harness-inheritance";

function harness(id: string, parentId: string | null = null): Harness {
  return {
    id,
    name: id,
    display_name: id,
    description: null,
    parent_harness_id: parentId,
    default_model_id: null,
    tags: [],
    capabilities: [],
    is_built_in: false,
    status: "active",
    created_at: "2026-08-07T00:00:00Z",
    updated_at: "2026-08-07T00:00:00Z",
    archived_at: null,
    deleted_at: null,
  };
}

describe("resolveHarnessInheritance", () => {
  it("distinguishes roots, direct parents, and multi-level ancestry", () => {
    const root = harness("root");
    const child = harness("child", root.id);
    const leaf = harness("leaf", child.id);
    const byId = new Map([root, child, leaf].map((item) => [item.id, item]));

    expect(resolveHarnessInheritance(root, byId)).toMatchObject({
      directParent: null,
      chain: [root],
      missingParentId: null,
      hasCycle: false,
    });
    expect(resolveHarnessInheritance(child, byId)).toMatchObject({
      directParent: root,
      chain: [root, child],
    });
    expect(resolveHarnessInheritance(leaf, byId)).toMatchObject({
      directParent: child,
      chain: [root, child, leaf],
    });
  });

  it("reports missing or deleted ancestors without following unsafe links", () => {
    const missingChild = harness("missing-child", "missing-parent");
    const deletedParent = { ...harness("deleted-parent"), status: "deleted" as const };
    const deletedChild = harness("deleted-child", deletedParent.id);
    const byId = new Map([[deletedParent.id, deletedParent]]);

    expect(resolveHarnessInheritance(missingChild, byId).missingParentId).toBe("missing-parent");
    expect(resolveHarnessInheritance(deletedChild, byId)).toMatchObject({
      directParent: deletedParent,
      missingParentId: deletedParent.id,
    });
  });

  it("bounds malformed cyclic ancestry", () => {
    const first = harness("first", "second");
    const second = harness("second", "first");
    const byId = new Map([first, second].map((item) => [item.id, item]));

    expect(resolveHarnessInheritance(first, byId).hasCycle).toBe(true);
  });
});
