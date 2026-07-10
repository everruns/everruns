import { render } from "@testing-library/react";
import SettingsPage from "@/app/(main)/settings/page";

const mockRedirect = jest.fn();
const mockReplace = jest.fn();

jest.mock("next/navigation", () => ({
  redirect: (...args: unknown[]) => mockRedirect(...args),
  useRouter: () => ({ replace: mockReplace }),
}));

describe("SettingsPage", () => {
  beforeEach(() => {
    mockRedirect.mockClear();
    mockReplace.mockClear();
  });

  it("redirects to the canonical Settings route during server rendering", () => {
    render(<SettingsPage />);

    expect(mockRedirect).toHaveBeenCalledWith("/settings/organization");
    expect(mockReplace).not.toHaveBeenCalled();
  });
});
