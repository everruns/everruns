import type { Harness } from "@/lib/api/types";
import {
  agentFilesSectionVisible,
  previewShowsFiles,
} from "@/components/agents/agent-files-visibility";

function harness(
  id: string,
  extras: Partial<Pick<Harness, "parent_harness_id" | "capabilities" | "initial_files">> = {},
): Harness {
  return {
    id,
    name: id,
    display_name: id,
    description: null,
    parent_harness_id: null,
    default_model_id: null,
    tags: [],
    capabilities: [],
    is_built_in: false,
    status: "active",
    created_at: "2026-08-07T00:00:00Z",
    updated_at: "2026-08-07T00:00:00Z",
    archived_at: null,
    deleted_at: null,
    ...extras,
  };
}

const fileSystem = { id: "session_file_system", features: ["file_system"] };
const skills = { id: "skills", dependencies: ["session_file_system"] };
const web = { id: "web_fetch", features: [] as string[] };

function visible(overrides: Partial<Parameters<typeof agentFilesSectionVisible>[0]> = {}): boolean {
  return agentFilesSectionVisible({
    capabilities: [],
    files: [],
    harnessId: "harness",
    harnesses: [harness("harness")],
    catalog: [fileSystem, skills, web],
    ...overrides,
  });
}

describe("agentFilesSectionVisible", () => {
  it("hides files when nothing contributes a filesystem", () => {
    expect(visible()).toBe(false);
    expect(visible({ capabilities: [{ ref: "web_fetch" }] })).toBe(false);
  });

  it("stays hidden while the catalog or harness list is still loading", () => {
    expect(visible({ catalog: undefined })).toBe(false);
    expect(visible({ harnesses: undefined })).toBe(false);
  });

  it("shows files when the agent, a dependency, or starter files enable them", () => {
    expect(visible({ capabilities: [{ ref: "session_file_system" }] })).toBe(true);
    expect(visible({ capabilities: [{ ref: "skills" }] })).toBe(true);
    expect(visible({ files: [{ path: "/notes.txt" }] })).toBe(true);
    expect(visible({ files: [{ path: "/notes.txt" }], catalog: undefined })).toBe(true);
  });

  it("shows files when the harness chain contributes them", () => {
    const parent = harness("parent", {
      capabilities: [{ ref: "session_file_system", config: {} }],
    });
    const child = harness("harness", { parent_harness_id: "parent" });
    expect(visible({ harnesses: [parent, child] })).toBe(true);
    expect(
      visible({
        harnesses: [
          harness("harness", {
            initial_files: [
              { path: "/seed.txt", content: "seed", encoding: "text", is_readonly: false },
            ],
          }),
        ],
      }),
    ).toBe(true);
  });

  it("keeps files when a selected capability or harness cannot be resolved", () => {
    expect(visible({ capabilities: [{ ref: "custom_pack" }] })).toBe(true);
    expect(visible({ harnessId: "missing" })).toBe(true);
    expect(visible({ harnesses: [harness("harness", { parent_harness_id: "gone" })] })).toBe(true);
  });
});

describe("previewShowsFiles", () => {
  it("follows the resolved filesystem feature and any files already present", () => {
    expect(previewShowsFiles({ features: ["secrets"], files: [] })).toBe(false);
    expect(previewShowsFiles({ features: [], files: [] })).toBe(false);
    expect(previewShowsFiles({ features: ["file_system"], files: [] })).toBe(true);
    expect(previewShowsFiles({ features: [], files: [{ path: "/a.txt" }] })).toBe(true);
    expect(previewShowsFiles({ features: undefined, files: [] })).toBe(true);
  });
});
