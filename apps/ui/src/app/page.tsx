import { redirect } from "next/navigation";

// Chats is the landing route: the first thing in the nav and the default
// destination for a user with no other intent
// (knowledge/ui/information-architecture.md).
export default function Home() {
  redirect("/chats");
}
