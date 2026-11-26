import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import ApplicationsPage from "@/app/(main)/applications/page";
import { createApplication } from "@/lib/api/applications";

const mockPush = jest.fn();
jest.mock("next/navigation", () => ({ useRouter: () => ({ push: mockPush }) }));
jest.mock("@/hooks", () => ({ usePageTitle: jest.fn() }));
jest.mock("@/hooks/use-applications", () => ({ useApplications: () => ({ data: [] }) }));
jest.mock("@/providers/org-provider", () => ({
  useOrg: () => ({ currentOrg: { public_id: "org_test", role: "owner" } }),
}));
jest.mock("@/providers/locale-provider", () => ({ useUiTranslations: () => (s: string) => s }));
jest.mock("@/lib/api/applications", () => ({ createApplication: jest.fn() }));
jest.mock("@/components/layout/page-layout", () => ({
  PageContainer: ({ children }: { children: React.ReactNode }) => <div>{children}</div>,
  PageBreadcrumb: () => null,
  PageMasthead: ({ title, actions }: { title: string; actions: React.ReactNode }) => (
    <>
      <h1>{title}</h1>
      {actions}
    </>
  ),
}));

it("creates an Application from the submit button and retains a manually chosen stable name", async () => {
  (createApplication as jest.Mock).mockResolvedValue({ id: "application_test" });
  render(<ApplicationsPage />);
  fireEvent.click(screen.getByRole("button", { name: "New application" }));
  fireEvent.change(screen.getByLabelText("Name"), { target: { value: "Content Studio" } });
  expect(screen.getByLabelText(/Stable name/)).toHaveValue("content-studio");
  fireEvent.change(screen.getByLabelText(/Stable name/), { target: { value: "editorial-studio" } });
  fireEvent.change(screen.getByLabelText("Name"), { target: { value: "Editorial Team" } });
  fireEvent.click(screen.getByRole("button", { name: "Create application" }));
  await waitFor(() =>
    expect(createApplication).toHaveBeenCalledWith(
      expect.objectContaining({
        name: "Editorial Team",
        slug: "editorial-studio",
        config: expect.objectContaining({ membership: "org_users" }),
      }),
    ),
  );
  await waitFor(() => expect(mockPush).toHaveBeenCalledWith("/applications/application_test"));
});
