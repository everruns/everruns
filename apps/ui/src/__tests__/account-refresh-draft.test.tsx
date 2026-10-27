import { useState } from "react";
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { QueryClientProvider, useQuery } from "@tanstack/react-query";
import { authQueryKeys } from "@/lib/auth-query-keys";
import { setActiveOrgId } from "@/lib/api/active-org";
import { createAppQueryClient } from "@/lib/query-client";

it("keeps mounted page drafts when account data refreshes after an organization switch", async () => {
  setActiveOrgId("org_a");
  const client = createAppQueryClient();
  client.setQueryData(authQueryKeys.user(), { id: "viewer", name: "Initial" });
  function Draft() {
    const [value, setValue] = useState("Saved");
    return (
      <input aria-label="Page draft" value={value} onChange={(e) => setValue(e.target.value)} />
    );
  }
  function AccountGate() {
    const query = useQuery({
      queryKey: authQueryKeys.user(),
      queryFn: async () => ({ id: "viewer", name: "Refreshed" }),
    });
    return query.isLoading ? (
      <p>Loading account</p>
    ) : (
      <>
        <p>{query.data?.name}</p>
        <Draft />
      </>
    );
  }
  render(
    <QueryClientProvider client={client}>
      <AccountGate />
    </QueryClientProvider>,
  );
  fireEvent.change(screen.getByLabelText("Page draft"), { target: { value: "Keep my edit" } });
  await act(async () => {
    setActiveOrgId("org_b");
    await client.invalidateQueries({ queryKey: authQueryKeys.user() });
  });
  await waitFor(() => expect(screen.getByText("Refreshed")).toBeInTheDocument());
  expect(screen.getByLabelText("Page draft")).toHaveValue("Keep my edit");
  client.clear();
  setActiveOrgId(null);
});
