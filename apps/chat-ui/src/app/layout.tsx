import type { ReactNode } from "react";
import "./style.css";
export const metadata = { title: "Work Chat", description: "Work with your agent" };
export default function Layout({ children }: { children: ReactNode }) {
  return (
    <html lang="en">
      <body>{children}</body>
    </html>
  );
}
