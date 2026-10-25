import KnowledgeIndexDetail from "@/app/(main)/knowledge-indexes/[indexId]/page";
import { redirect } from "next/navigation";
jest.mock("next/navigation", () => ({ redirect: jest.fn() }));
it("redirects legacy index details to unified Knowledge", () => {
  KnowledgeIndexDetail();
  expect(redirect).toHaveBeenCalledWith("/knowledge");
});
