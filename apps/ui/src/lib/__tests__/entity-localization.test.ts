import { getDisplayName } from "@/lib/entity-lifecycle";
import { localizedEntityDescription } from "@/lib/entity-localization";

describe("resource display localization", () => {
  const entity = {
    name: "customer-support",
    display_name: "Support",
    description: "Help customers",
    localizations: {
      "uk-ua": { display_name: "Підтримка в Україні" },
      uk: { display_name: "Підтримка", description: "Допомога клієнтам" },
      de: { display_name: "Kundensupport" },
      en: { description: "Customer help" },
    },
  };
  it("resolves exact locale and language per field", () => {
    expect(getDisplayName(entity, "UK_UA")).toBe("Підтримка в Україні");
    expect(localizedEntityDescription(entity, "uk-UA")).toBe("Допомога клієнтам");
    expect(getDisplayName(entity, "de-AT")).toBe("Kundensupport");
    expect(localizedEntityDescription(entity, "de-AT")).toBe("Customer help");
  });
  it("uses base values without changing stable addresses", () => {
    expect(getDisplayName(entity, "fr-FR")).toBe("Support");
    expect(entity.name).toBe("customer-support");
    expect(getDisplayName({ name: "slug" }, "uk")).toBe("slug");
    expect(getDisplayName(null, "uk")).toBe("");
  });
});
