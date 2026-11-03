"use client";
import { useEffect, useRef, type MouseEvent } from "react";
import { usePathname, useRouter } from "next/navigation";
import { LinkButton } from "@/components/ui/button";
import { useUiTranslations } from "@/providers/locale-provider";
import { FileNavigationContext } from "@/components/files/file-source";
import { OrgHome } from "@/components/home/org-home";
import { SessionProvider } from "@/app/(main)/sessions/[sessionId]/session-context";
import { SessionLayoutContent } from "@/app/(main)/sessions/[sessionId]/layout";
import Trace from "@/app/(main)/sessions/[sessionId]/trace/page";
import Approvals from "@/app/(main)/sessions/[sessionId]/approvals/page";
import Work from "@/app/(main)/sessions/[sessionId]/work/page";
import Events from "@/app/(main)/sessions/[sessionId]/events/page";
import Files from "@/app/(main)/sessions/[sessionId]/files/page";
import Cost from "@/app/(main)/sessions/[sessionId]/cost/page";
import { AgentView, HarnessView, CapabilityView, KnowledgeView } from "./entity-view";

const tabs: Record<string, React.ComponentType> = {
  trace: Trace,
  approvals: Approvals,
  work: Work,
  events: Events,
  files: Files,
  cost: Cost,
};
export function McpWorkspacePage() {
  const router = useRouter();
  const ui = useUiTranslations();
  const pathname = usePathname();
  const root = useRef<HTMLDivElement>(null);
  const [, section, id, tab] = pathname.slice(1).split("/");
  const full = section === "home" || section === "sessions";
  useEffect(() => {
    if (!root.current) return;
    const observer = new ResizeObserver(() =>
      window.parent.postMessage(
        { type: "everruns:size", height: full ? 760 : root.current?.scrollHeight },
        "*",
      ),
    );
    observer.observe(root.current);
    return () => observer.disconnect();
  }, [full]);
  // Keep session tabs in this workspace; entity titles and all other destinations
  // open the real application through the host's approved link handler.
  function navigate(event: MouseEvent) {
    const anchor = (event.target as Element).closest<HTMLAnchorElement>("a[href]");
    if (!anchor) return;
    const url = new URL(anchor.href, location.origin);
    if (!["https:", "http:"].includes(url.protocol)) {
      event.preventDefault();
      return;
    }
    if (
      url.origin === location.origin &&
      url.pathname === location.pathname &&
      url.search === location.search &&
      url.hash
    )
      return;
    event.preventDefault();
    event.stopPropagation();
    if (
      anchor.target !== "_blank" &&
      url.origin === location.origin &&
      /^\/sessions\/[^/]+(?:\/(trace|approvals|work|events|files|cost))?$/.test(url.pathname)
    ) {
      router.push(
        `/mcp-app${url.pathname}${url.pathname.split("/").length === 3 ? "/trace" : ""}${url.search}`,
      );
    } else {
      window.parent.postMessage({ type: "everruns:open-link", url: url.href }, "*");
    }
  }
  const Tab = tabs[tab] ?? Trace;
  return (
    <div
      ref={root}
      data-mcp-workspace={section}
      onClickCapture={navigate}
      className={
        full
          ? "h-screen overflow-auto bg-background text-foreground"
          : "mx-auto max-w-xl bg-background p-4 text-foreground"
      }
    >
      {section === "home" && <OrgHome />}
      {section === "sessions" && id && (
        <FileNavigationContext
          value={{ browserUrls: false, externalPreviewHref: `/sessions/${id}/files` }}
        >
          <SessionProvider key={id} sessionId={id}>
            <SessionLayoutContent
              sessionId={id}
              onForked={(sessionId) =>
                window.parent.postMessage(
                  {
                    type: "everruns:open-link",
                    url: new URL(`/chats/${sessionId}`, location.origin).href,
                  },
                  "*",
                )
              }
              credentialsAction={
                <LinkButton
                  variant="outline"
                  size="sm"
                  target="_blank"
                  href={`/sessions/${id}/trace`}
                  title={ui("Open in Everruns")}
                >
                  {ui("Credentials")}
                </LinkButton>
              }
            >
              <Tab />
            </SessionLayoutContent>
          </SessionProvider>
        </FileNavigationContext>
      )}
      {section === "agents" && id && <AgentView id={id} />}
      {section === "harnesses" && id && <HarnessView id={id} />}
      {section === "capabilities" && id && <CapabilityView id={id} />}
      {section === "knowledge" && id && <KnowledgeView id={id} />}
    </div>
  );
}
