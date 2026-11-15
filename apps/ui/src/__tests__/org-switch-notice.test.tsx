/**
 * A switch the app makes by itself to open a page is announced once, with a
 * way back to the org the user was in.
 */
import { render } from "@testing-library/react";
import { OrgSwitchNotice } from "@/components/layout/org-switch-notice";
import type { AutoOrgSwitch } from "@/providers/org-provider";
import type { OrganizationMembership } from "@/lib/api/types";

const PERSONAL: OrganizationMembership = {
  public_id: "org_personal",
  name: "Personal",
  role: "owner",
};
const ALICE: OrganizationMembership = {
  public_id: "org_alice",
  name: "Personal Alice",
  role: "member",
};

let mockAutoSwitch: AutoOrgSwitch | null = null;
const mockSetCurrentOrg = jest.fn();
jest.mock("@/providers/org-provider", () => ({
  useOrg: () => ({ autoSwitch: mockAutoSwitch, setCurrentOrg: mockSetCurrentOrg }),
}));
jest.mock("@/providers/locale-provider", () => ({
  useUiTranslations: () => (message: string, values?: Record<string, unknown>) =>
    values ? message.replace(/\{(\w+)\}/g, (_, key: string) => String(values[key])) : message,
}));

beforeEach(() => {
  mockAutoSwitch = null;
  mockSetCurrentOrg.mockClear();
});

test("says which org it switched to and offers the way back", () => {
  const notify = jest.fn();
  mockAutoSwitch = { id: 1, from: PERSONAL, to: ALICE };

  render(<OrgSwitchNotice notify={notify} />);

  expect(notify).toHaveBeenCalledTimes(1);
  const toast = notify.mock.calls[0][0];
  expect(toast.title).toBe("Switched to Personal Alice");
  expect(toast.body).toContain("belongs to Personal Alice");
  expect(toast.action.label).toBe("Back to Personal");
  toast.action.onClick();
  expect(mockSetCurrentOrg).toHaveBeenCalledWith(PERSONAL);
});

test("announces each switch once", () => {
  const notify = jest.fn();
  mockAutoSwitch = { id: 1, from: PERSONAL, to: ALICE };
  const { rerender } = render(<OrgSwitchNotice notify={notify} />);
  rerender(<OrgSwitchNotice notify={notify} />);
  expect(notify).toHaveBeenCalledTimes(1);

  mockAutoSwitch = { id: 2, from: ALICE, to: PERSONAL };
  rerender(<OrgSwitchNotice notify={notify} />);
  expect(notify).toHaveBeenCalledTimes(2);
});

test("has no way back when the previous org is unknown", () => {
  const notify = jest.fn();
  mockAutoSwitch = { id: 1, from: null, to: ALICE };
  render(<OrgSwitchNotice notify={notify} />);
  expect(notify.mock.calls[0][0].action).toBeUndefined();
});

test("stays quiet without a switch", () => {
  const notify = jest.fn();
  render(<OrgSwitchNotice notify={notify} />);
  expect(notify).not.toHaveBeenCalled();
});
