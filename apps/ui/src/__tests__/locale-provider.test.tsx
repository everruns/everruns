import { fireEvent, render, screen, act } from "@testing-library/react";
import { LocaleProvider, useLocale, useUiTranslations } from "@/providers/locale-provider";
import { LanguageSettings } from "@/components/settings/language-settings";

function Sample() {
  const { backendLocale } = useLocale();
  const ui = useUiTranslations();
  return (
    <>
      <LanguageSettings />
      <p>{ui("Agents")}</p>
      <output>{backendLocale}</output>
    </>
  );
}

describe("interface language preference", () => {
  beforeEach(() => {
    localStorage.clear();
    Object.defineProperty(navigator, "languages", { configurable: true, value: ["en-US"] });
  });
  it("switches immediately, persists, and keeps the runtime language separate", () => {
    const view = render(
      <LocaleProvider>
        <Sample />
      </LocaleProvider>,
    );
    fireEvent.change(screen.getByLabelText("Interface language"), { target: { value: "uk" } });
    expect(screen.getByText("Агенти")).toBeInTheDocument();
    expect(screen.getByText("en-US")).toBeInTheDocument();
    expect(document.documentElement.lang).toBe("uk");
    view.unmount();
    render(
      <LocaleProvider>
        <Sample />
      </LocaleProvider>,
    );
    expect(screen.getByText("Агенти")).toBeInTheDocument();
    act(() => {
      localStorage.setItem("everruns.ui-language", "en");
      window.dispatchEvent(new StorageEvent("storage"));
    });
    expect(screen.getByText("Agents")).toBeInTheDocument();
  });
});

describe.each([
  ["fr", "Agents", "Langue de l’interface"],
  ["de", "Agenten", "Oberflächensprache"],
  ["es", "Agentes", "Idioma de la interfaz"],
  ["zh-CN", "智能体", "界面语言"],
  ["en-GB", "Agents", "Interface language"],
])("%s interface preference", (locale, agents, label) => {
  beforeEach(() => {
    localStorage.clear();
    Object.defineProperty(navigator, "languages", { configurable: true, value: ["en-US"] });
  });
  it("persists an explicit choice and leaves execution locale unchanged", () => {
    const view = render(
      <LocaleProvider>
        <Sample />
      </LocaleProvider>,
    );
    fireEvent.change(screen.getByLabelText("Interface language"), { target: { value: locale } });
    expect(screen.getByText(agents)).toBeInTheDocument();
    expect(screen.getByLabelText(label)).toHaveValue(locale);
    expect(screen.getByText("en-US")).toBeInTheDocument();
    view.unmount();
    render(
      <LocaleProvider>
        <Sample />
      </LocaleProvider>,
    );
    expect(screen.getByLabelText(label)).toHaveValue(locale);
  });
});
