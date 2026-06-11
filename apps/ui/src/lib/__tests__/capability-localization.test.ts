import {
  localizedCapabilityDescription,
  localizedCapabilityName,
  localizedConfigDescription,
  localizedConfigSchema,
} from "@/lib/capability-localization";

const capability = {
  name: "Memory",
  description: "Mount Memories into the session workspace.",
  config_schema: {
    type: "object",
    properties: {
      mounts: {
        type: "array",
        title: "Mounts",
        description: "Memories mounted into /workspace.",
        items: {
          type: "object",
          properties: {
            mode: {
              type: "string",
              title: "Access mode",
              oneOf: [
                { const: "readonly", title: "Read-only" },
                { const: "readwrite", title: "Read-write" },
              ],
              default: "readonly",
            },
          },
        },
      },
      flavor: {
        type: "string",
        enum: ["a", "b"],
      },
    },
  },
  localizations: {
    en: {
      config_description: "Choose which Memories are mounted.",
    },
    uk: {
      name: "Пам'ять",
      description: "Підключає Пам'яті до робочого простору сесії.",
      config_description: "Визначає, які Пам'яті монтуються.",
      config_overlay: {
        properties: {
          mounts: {
            title: "Монтування",
            description: "Пам'яті, що монтуються у /workspace.",
            items: {
              properties: {
                mode: {
                  title: "Режим доступу",
                  enum_labels: { readonly: "Лише читання", readwrite: "Читання й запис" },
                },
              },
            },
          },
          flavor: {
            enum_labels: { a: "А" },
          },
          unknown_field: { title: "Невідоме" },
        },
      },
    },
  },
};

describe("capability localization", () => {
  it("resolves uk name/description and falls back to base for en", () => {
    expect(localizedCapabilityName(capability, "uk")).toBe("Пам'ять");
    expect(localizedCapabilityName(capability, "en")).toBe("Memory");
    expect(localizedCapabilityDescription(capability, "uk")).toBe(
      "Підключає Пам'яті до робочого простору сесії.",
    );
    expect(localizedCapabilityDescription(capability, "en")).toBe(
      "Mount Memories into the session workspace.",
    );
  });

  it("falls back to base strings when there are no localizations", () => {
    const bare = { name: "Web Fetch", description: "Fetches URLs." };
    expect(localizedCapabilityName(bare, "uk")).toBe("Web Fetch");
    expect(localizedCapabilityDescription(bare, "uk")).toBe("Fetches URLs.");
  });

  it("resolves config description with en fallback", () => {
    expect(localizedConfigDescription(capability, "uk")).toBe("Визначає, які Пам'яті монтуються.");
    expect(localizedConfigDescription({ ...capability, localizations: { en: { config_description: "Only en." } } }, "uk")).toBe(
      "Only en.",
    );
    expect(localizedConfigDescription({}, "uk")).toBeUndefined();
  });

  it("merges uk overlay into the schema without touching validation keywords", () => {
    const merged = localizedConfigSchema(capability, "uk") as any;

    const mounts = merged.properties.mounts;
    expect(mounts.title).toBe("Монтування");
    expect(mounts.description).toBe("Пам'яті, що монтуються у /workspace.");
    expect(mounts.type).toBe("array");

    const mode = mounts.items.properties.mode;
    expect(mode.title).toBe("Режим доступу");
    expect(mode.default).toBe("readonly");
    expect(mode.oneOf).toEqual([
      { const: "readonly", title: "Лише читання" },
      { const: "readwrite", title: "Читання й запис" },
    ]);

    // Plain enums are rewritten to oneOf so labels render; unmapped values
    // keep their raw value as the label.
    expect(merged.properties.flavor.oneOf).toEqual([
      { const: "a", title: "А" },
      { const: "b", title: "b" },
    ]);
    expect(merged.properties.flavor.enum).toBeUndefined();

    // Overlay paths that do not exist in the schema are ignored.
    expect(merged.properties.unknown_field).toBeUndefined();
  });

  it("returns the base schema untouched for en", () => {
    expect(localizedConfigSchema(capability, "en")).toBe(capability.config_schema);
    expect(localizedConfigSchema({}, "uk")).toBeUndefined();
  });
});
