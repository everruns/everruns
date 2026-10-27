import { render, waitFor } from "@testing-library/react";
import { PageChatProvider } from "@/test-utils/page-chat-provider";
import { Dialog, DialogContent, DialogTitle, DialogDescription } from "@/components/ui/dialog";
import { useTextPageFormDraft } from "@/components/chat-mode/use-text-page-form-draft";
import type { ChatModeState } from "@/components/chat-mode/chat-mode-context";
import type { PageDraftAdapter } from "@/components/chat-mode/page-draft";

it("masks the underlying writable form while a credential popup is mounted and restores it on close", async () => {
  global.structuredClone = (v) => JSON.parse(JSON.stringify(v));
  const entries = new Map<string, { adapter: PageDraftAdapter; priority: number }>();
  const mode: ChatModeState = {
    page: { route: "/agents/agent_1", label: "Agent" },
    pageContainer: { current: null },
    open: true,
    setOpen: () => {},
    actions: [],
    recordAction: () => {},
    registerPage: (_, adapter, priority = 0) => {
      const key = crypto.randomUUID();
      entries.set(key, { adapter: adapter!, priority });
      return () => {
        entries.delete(key);
      };
    },
  };
  const active = () => [...entries.values()].sort((a, b) => b.priority - a.priority)[0].adapter;
  function Form({ open }: { open: boolean }) {
    useTextPageFormDraft({
      fields: ["name"],
      values: { name: "Safe draft" },
      entityType: "agent",
      label: "Agent",
      baseline: "saved",
      dirty: false,
      writable: true,
      apply: () => {},
    });
    return (
      <Dialog open={open}>
        <DialogContent chatProtected="Credentials">
          <DialogTitle>Set key</DialogTitle>
          <DialogDescription>Manual setup</DialogDescription>
          <input type="password" defaultValue="fixture-secret" />
        </DialogContent>
      </Dialog>
    );
  }
  const tree = (open: boolean) => (
    <PageChatProvider value={mode}>
      <Form open={open} />
    </PageChatProvider>
  );
  const { rerender } = render(tree(false));
  expect(active().describe().writable).toBe(true);
  rerender(tree(true));
  await waitFor(() => expect(active().describe().writable).toBe(false));
  const snapshot = active().read(active().describe().binding);
  expect(JSON.stringify(snapshot)).not.toContain("fixture-secret");
  expect(snapshot.entity_type).toBe("manual_dialog");
  expect(() =>
    active().patch(snapshot.binding, snapshot.revision, { name: "Unintended edit" }),
  ).toThrow(/read-only/);
  rerender(tree(false));
  await waitFor(() => expect(active().describe().writable).toBe(true));
});
