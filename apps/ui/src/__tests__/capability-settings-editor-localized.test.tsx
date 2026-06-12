import { render, screen } from "@testing-library/react";
import {
  CapabilitySettingsEditor,
  translateValidationErrors,
} from "@/components/agents/capability-settings-editor";
import { formatMessage } from "@/lib/i18n";
import type { Capability } from "@/lib/api/types";

jest.mock("@/providers/locale-provider", () => ({
  useLocale: () => ({
    locale: "uk",
    backendLocale: "uk-UA",
    t: (key: never) => String(key),
  }),
}));

function localizedCapability(): Capability {
  return {
    id: "web_fetch",
    name: "Web Fetch",
    description: "Fetches web content.",
    status: "available",
    config_schema: {
      type: "object",
      properties: {
        enable_file_download: {
          type: "boolean",
          title: "Allow saving fetched files",
          description: "Let the tool save responses to the workspace.",
          default: false,
        },
      },
    },
    localizations: {
      en: {
        config_description: "Controls whether fetched responses may be saved.",
      },
      uk: {
        name: "Отримання вебвмісту",
        description: "Отримує вміст за URL-адресою.",
        config_description: "Визначає, чи можна зберігати отримані відповіді.",
        config_overlay: {
          properties: {
            enable_file_download: {
              title: "Дозволити збереження файлів",
              description: "Дозволяє зберігати відповіді у робочий простір.",
            },
          },
        },
      },
    },
  };
}

describe("CapabilitySettingsEditor with uk locale", () => {
  it("renders localized titles, descriptions, and the config summary line", () => {
    render(
      <CapabilitySettingsEditor
        capability={localizedCapability()}
        config={{}}
        onChange={jest.fn()}
      />,
    );

    expect(
      screen.getByText("Визначає, чи можна зберігати отримані відповіді."),
    ).toBeInTheDocument();
    expect(screen.getByText("Дозволити збереження файлів")).toBeInTheDocument();
    expect(screen.getByText("Дозволяє зберігати відповіді у робочий простір.")).toBeInTheDocument();
    // Base English labels are replaced by the overlay.
    expect(screen.queryByText("Allow saving fetched files")).not.toBeInTheDocument();
  });
});

describe("translateValidationErrors", () => {
  it("maps known AJV keywords to localized catalog messages", () => {
    const errors = translateValidationErrors("uk", [
      { name: "required", message: "is a required property", stack: ".x is required" },
      { name: "pattern", params: { pattern: "^mem_" }, message: "must match pattern", stack: "" },
      { name: "minimum", params: { limit: 1 }, message: "must be >= 1", stack: "" },
    ]);

    expect(errors[0].message).toBe(formatMessage("uk", "validation_required"));
    expect(errors[1].message).toBe(formatMessage("uk", "validation_pattern"));
    expect(errors[2].message).toBe(formatMessage("uk", "validation_minimum", { value: 1 }));
    // stack mirrors message so both render paths show the localized text
    expect(errors[2].stack).toBe(errors[2].message);
  });

  it("keeps the original message for unknown keywords", () => {
    const errors = translateValidationErrors("uk", [
      { name: "customKeyword", message: "原文 stays", stack: "原文 stays" },
    ]);
    expect(errors[0].message).toBe("原文 stays");
  });
});
