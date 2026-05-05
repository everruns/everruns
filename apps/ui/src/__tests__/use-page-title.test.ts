import { renderHook } from "@testing-library/react";
import { usePageTitle } from "@/hooks/use-page-title";

describe("usePageTitle", () => {
  beforeEach(() => {
    document.title = "initial";
  });

  it("sets document.title to the formatted value on mount", () => {
    renderHook(() => usePageTitle("Agents"));
    expect(document.title).toBe("Agents · Everruns");
  });

  it("joins multiple segments with the separator", () => {
    renderHook(() => usePageTitle("New Agent", "Agents"));
    expect(document.title).toBe("New Agent · Agents · Everruns");
  });

  it("treats null/undefined segments as loading and skips them", () => {
    renderHook(() => usePageTitle(null, "Agent"));
    expect(document.title).toBe("Agent · Everruns");
  });

  it("updates document.title when arguments change", () => {
    const { rerender } = renderHook(
      ({ name }: { name: string | null }) => usePageTitle(name, "Agent"),
      {
        initialProps: { name: null },
      },
    );
    expect(document.title).toBe("Agent · Everruns");

    rerender({ name: "My Bot" });
    expect(document.title).toBe("My Bot · Agent · Everruns");
  });

  it("restores the previous title on unmount", () => {
    document.title = "Outside · Everruns";
    const { unmount } = renderHook(() => usePageTitle("Inside"));
    expect(document.title).toBe("Inside · Everruns");

    unmount();
    expect(document.title).toBe("Outside · Everruns");
  });
});
