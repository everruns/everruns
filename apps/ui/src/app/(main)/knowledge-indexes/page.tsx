import { redirect } from "next/navigation";
export default function Page(): React.ReactNode {
  redirect("/knowledge");
  return null;
}
