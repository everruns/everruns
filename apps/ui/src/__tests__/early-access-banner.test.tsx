// Regression test for EVE-794: the early-access banner used to return null while
// the dismissed preference loaded, then insert a 44px (h-11) row after the
// request resolved — shifting the whole app shell (CLS ~0.024). The dismissal
// decision is now seeded synchronously from a localStorage cache so first paint
// is already correct: an undismissed user gets the banner as part of the initial
// stable layout, and a dismissed user gets neither a flash nor a leftover gap.

import { render, screen, fireEvent } from "@testing-library/react";
import type { UserPreference } from "@/lib/api/user-preferences";

const CACHE_KEY = "everruns:ui:early-access-banner-dismissed";

// Controllable preference-query result. `undefined` models the still-loading
// state; `null` an unset preference; an object a stored value.
let preferenceData: UserPreference | null | undefined;
const mockMutate = jest.fn();

jest.mock("@/hooks/use-user-preferences", () => ({
  useUserPreference: () => ({ data: preferenceData }),
  useSetUserPreference: () => ({ mutate: mockMutate }),
}));

import { EarlyAccessBanner } from "@/components/layout/early-access-banner";

function getBanner(): HTMLElement | null {
  // The banner row carries the fixed 44px height; its absence means no shell gap.
  return document.querySelector(".h-11");
}

beforeEach(() => {
  preferenceData = undefined;
  mockMutate.mockClear();
  window.localStorage.clear();
});

describe("EarlyAccessBanner (EVE-794)", () => {
  it("renders the banner during loading for a user with no cached dismissal", () => {
    preferenceData = undefined; // query still loading
    render(<EarlyAccessBanner />);

    expect(getBanner()).not.toBeNull();
    expect(screen.getByText("Early access.")).toBeInTheDocument();
  });

  it("does not render during loading when the local cache says dismissed", () => {
    window.localStorage.setItem(CACHE_KEY, "true");
    preferenceData = undefined; // query still loading
    render(<EarlyAccessBanner />);

    // No flash and no reserved gap for a dismissed user, even before the server
    // preference resolves.
    expect(getBanner()).toBeNull();
  });

  it("renders the banner when the preference is unset", () => {
    preferenceData = null;
    render(<EarlyAccessBanner />);

    expect(getBanner()).not.toBeNull();
    expect(screen.getByRole("link", { name: /Follow along on GitHub/i })).toHaveAttribute(
      "href",
      "https://github.com/everruns/everruns",
    );
    expect(screen.getByRole("button", { name: "Dismiss" })).toBeInTheDocument();
  });

  it("does not render (no gap) when the preference is dismissed", () => {
    preferenceData = { key: "ui.early_access_banner.dismissed", value: true, updated_at: "" };
    render(<EarlyAccessBanner />);

    expect(getBanner()).toBeNull();
    // The resolved server value reconciles the local cache for the next load.
    expect(window.localStorage.getItem(CACHE_KEY)).toBe("true");
  });

  it("does not shift: an undismissed banner stays present as the query resolves", () => {
    preferenceData = undefined;
    const { rerender } = render(<EarlyAccessBanner />);
    expect(getBanner()).not.toBeNull();

    // Server resolves to unset — the banner must remain, no insert/collapse.
    preferenceData = null;
    rerender(<EarlyAccessBanner />);
    expect(getBanner()).not.toBeNull();
  });

  it("does not shift: a cached-dismissed user stays hidden as the query resolves", () => {
    window.localStorage.setItem(CACHE_KEY, "true");
    preferenceData = undefined;
    const { rerender } = render(<EarlyAccessBanner />);
    expect(getBanner()).toBeNull();

    preferenceData = { key: "ui.early_access_banner.dismissed", value: true, updated_at: "" };
    rerender(<EarlyAccessBanner />);
    expect(getBanner()).toBeNull();
  });

  it("hides immediately, caches, and persists the dismissal on click", () => {
    preferenceData = null;
    render(<EarlyAccessBanner />);

    fireEvent.click(screen.getByRole("button", { name: "Dismiss" }));

    // Hidden immediately, before the server write settles.
    expect(getBanner()).toBeNull();
    // Cached synchronously so a reload stays dismissed.
    expect(window.localStorage.getItem(CACHE_KEY)).toBe("true");
    // Persisted to the server preference (source of truth).
    expect(mockMutate).toHaveBeenCalledWith({
      key: "ui.early_access_banner.dismissed",
      value: true,
    });
  });
});
