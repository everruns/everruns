import KnowledgeIndexesPage from "@/app/(main)/knowledge-indexes/page";
import { redirect } from "next/navigation";
jest.mock("next/navigation", () => ({ redirect: jest.fn() }));
it("redirects the undeployed legacy index page to Knowledge", () => {
  KnowledgeIndexesPage();
  expect(redirect).toHaveBeenCalledWith("/knowledge");
});
