import { PageViewRegistryContext } from "@/components/chat-mode/page-view-state";
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { useState } from "react";
import { ChatModeContext, type ChatModeState } from "@/components/chat-mode/chat-mode-context";
import { Dialog, DialogContent, DialogTitle, DialogTrigger } from "@/components/ui/dialog";
import { Drawer, DrawerContent, DrawerTitle } from "@/components/ui/drawer";

function Workspace({
  drawer = false,
  pageOverlay = true,
}: {
  drawer?: boolean;
  pageOverlay?: boolean;
}) {
  const [open, setOpen] = useState(false);
  const [container, setContainer] = useState<HTMLDivElement | null>(null);
  const [drawerOpen, setDrawerOpen] = useState(false);
  const mode: ChatModeState = {
    open,
    setOpen,
    page: { route: "/skills", label: "Skills" },
    pageContainer: { current: container },
    pageContainerElement: container,
    registerPage: () => () => {},
    actions: [],
    recordAction: () => {},
  };
  return (
    <ChatModeContext.Provider value={mode}>
      <button onClick={() => setOpen(true)}>Open Chat</button>
      <input aria-label="Chat composer" />
      <PageViewRegistryContext.Provider
        value={pageOverlay ? { route: "/skills", register: () => () => {} } : null}
      >
        <div ref={setContainer} data-testid="page">
          {drawer ? (
            <>
              <button onClick={() => setDrawerOpen(true)}>New skill</button>
              <Drawer open={drawerOpen} onOpenChange={setDrawerOpen}>
                <DrawerContent>
                  <DrawerTitle>Skill draft</DrawerTitle>
                  <input aria-label="Skill instructions" defaultValue="" />
                </DrawerContent>
              </Drawer>
            </>
          ) : (
            <Dialog>
              <DialogTrigger>New skill</DialogTrigger>
              <DialogContent>
                <DialogTitle>Skill draft</DialogTitle>
                <input aria-label="Skill instructions" defaultValue="" />
              </DialogContent>
            </Dialog>
          )}
        </div>
      </PageViewRegistryContext.Provider>
    </ChatModeContext.Provider>
  );
}

it.each([false, true])(
  "keeps overlay drafts mounted while Chat is opened and focused (drawer=%s)",
  async (drawer) => {
    render(<Workspace drawer={drawer} />);
    fireEvent.click(screen.getByText("New skill"));
    const input = await screen.findByLabelText("Skill instructions");
    fireEvent.change(input, { target: { value: "Unsaved instructions" } });
    expect(screen.getByTestId("page")).toContainElement(input);
    fireEvent.click(screen.getByText("Open Chat"));
    act(() => {
      screen.getByLabelText("Chat composer").focus();
    });
    expect(screen.getByLabelText("Skill instructions")).toBe(input);
    expect(input).toHaveValue("Unsaved instructions");
    await waitFor(() => expect(screen.getByLabelText("Chat composer")).toHaveFocus());
    fireEvent.click(screen.getByRole("button", { name: /^Close$/ }));
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
  },
);

it.each([false, true])(
  "retains global overlay behavior outside the right page (drawer=%s)",
  async (drawer) => {
    render(<Workspace drawer={drawer} pageOverlay={false} />);
    fireEvent.click(screen.getByText("New skill"));
    const input = await screen.findByLabelText("Skill instructions");
    fireEvent.click(screen.getByText("Open Chat"));
    expect(screen.getByTestId("page")).not.toContainElement(input);
    // JSDOM allows focus() on inert nodes. The primitive's modal isolation is
    // the observable contract here; only page-scoped dialogs may release it.
    await waitFor(() =>
      expect(
        screen.getByLabelText("Chat composer").closest("[data-base-ui-inert]"),
      ).toHaveAttribute("aria-hidden", "true"),
    );
  },
);
