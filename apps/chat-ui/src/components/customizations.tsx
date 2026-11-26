"use client";

import { apiPath } from "./api-path";
// Customizations: the plugins in this person's conversations with the agent.
// A plugin is one row; its skills and MCP servers are never listed on their
// own. Choices apply to new conversations. The channel decides whether people
// may add approved plugins and turn off the agent's own
// (knowledge/integrations/plugins.md, "Personal plugins").
import { useEffect, useState } from "react";

type Origin = "agent" | "added" | "available";
type Plugin = {
  id: string;
  name: string;
  display_name?: string | null;
  description: string;
  environment: string;
  origin: Origin;
  enabled: boolean;
  fits: boolean;
  needs_admin_setup: boolean;
};
type PluginList = { allow_adding: boolean; allow_turning_off: boolean; plugins: Plugin[] };

const ENVIRONMENT: Record<string, string> = {
  conversation: "Chat only",
  workspace: "Needs files",
  "virtual-shell": "Needs a shell",
  sandbox: "Needs a sandbox",
};

async function request(path: string, init?: RequestInit): Promise<PluginList> {
  const response = await fetch(apiPath(`chat/plugins${path}`), { cache: "no-store", ...init });
  if (response.status === 401) throw new Error("Sign in to continue");
  if (!response.ok) {
    const data = await response.json().catch(() => null);
    throw new Error(
      typeof data?.error === "string"
        ? data.error
        : typeof data?.detail === "string"
          ? data.detail
          : "The request could not be completed",
    );
  }
  return response.json();
}

export function Customizations({ onBack }: { onBack: () => void }) {
  const [list, setList] = useState<PluginList | null>(null),
    [error, setError] = useState<string | null>(null),
    [busy, setBusy] = useState<string | null>(null),
    [search, setSearch] = useState("");
  useEffect(() => {
    request("")
      .then(setList)
      .catch((reason: Error) => setError(reason.message));
  }, []);
  const choose = async (plugin: Plugin, enabled: boolean) => {
    setBusy(plugin.id);
    setError(null);
    try {
      setList(
        await request(`/${encodeURIComponent(plugin.id)}`, {
          method: "PUT",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ enabled }),
        }),
      );
    } catch (reason) {
      setError((reason as Error).message);
    } finally {
      setBusy(null);
    }
  };
  const query = search.trim().toLowerCase();
  const matches = (plugin: Plugin) =>
    !query ||
    `${plugin.display_name ?? ""} ${plugin.name} ${plugin.description}`
      .toLowerCase()
      .includes(query);
  const yours = list?.plugins.filter((plugin) => plugin.origin !== "available" && matches(plugin));
  const available = list?.plugins.filter(
    (plugin) => plugin.origin === "available" && matches(plugin),
  );
  return (
    <main className="customizations">
      <header>
        <button className="mobile-back" aria-label="Back to the conversation" onClick={onBack}>
          ←
        </button>
        <div>
          <strong>Customizations</strong>
          <small>Plugins in your new conversations</small>
        </div>
      </header>
      <div className="customizations-body">
        <input
          aria-label="Search plugins"
          className="conversation-search"
          placeholder="Search plugins…"
          value={search}
          onChange={(event) => setSearch(event.target.value)}
        />
        {error && <p className="customizations-error">{error}</p>}
        {!list && !error && <p className="customizations-muted">Loading…</p>}
        {list && (
          <>
            <section>
              <h2>Your plugins</h2>
              {yours && yours.length > 0 ? (
                <div className="plugin-rows">
                  {yours.map((plugin) => {
                    const locked =
                      plugin.origin === "agent" && !list.allow_turning_off
                        ? "Comes with the agent"
                        : null;
                    return (
                      <div key={plugin.id} className="plugin-row">
                        <PluginIcon plugin={plugin} />
                        <div className="plugin-text">
                          <div className="plugin-title">
                            <strong>{plugin.display_name ?? plugin.name}</strong>
                            <span className="pill">
                              {plugin.origin === "agent" ? "From the agent" : "Added by you"}
                            </span>
                            {plugin.needs_admin_setup && (
                              <span className="pill warn">Waiting for an admin</span>
                            )}
                          </div>
                          {plugin.description && <p>{plugin.description}</p>}
                        </div>
                        <label className="toggle" title={locked ?? undefined}>
                          <input
                            type="checkbox"
                            role="switch"
                            aria-label={`${plugin.display_name ?? plugin.name} in new conversations`}
                            checked={plugin.enabled}
                            disabled={Boolean(locked) || busy !== null}
                            onChange={(event) => void choose(plugin, event.target.checked)}
                          />
                          <span />
                        </label>
                      </div>
                    );
                  })}
                </div>
              ) : (
                <p className="customizations-muted">
                  {query ? "No plugins match." : "This agent has no plugins yet."}
                </p>
              )}
            </section>
            {list.allow_adding && (
              <section>
                <h2>Available to add</h2>
                {available && available.length > 0 ? (
                  <div className="plugin-grid">
                    {available.map((plugin) => {
                      const blocked = !plugin.fits
                        ? "Can't run here"
                        : plugin.needs_admin_setup
                          ? "Waiting for an admin"
                          : null;
                      return (
                        <div key={plugin.id} className="plugin-card">
                          <div className="plugin-title">
                            <PluginIcon plugin={plugin} />
                            <strong>{plugin.display_name ?? plugin.name}</strong>
                          </div>
                          {plugin.description && <p>{plugin.description}</p>}
                          <div className="plugin-card-footer">
                            <span className="pill">
                              {ENVIRONMENT[plugin.environment] ?? plugin.environment}
                            </span>
                            {blocked ? (
                              <span className="customizations-muted">{blocked}</span>
                            ) : (
                              <button
                                disabled={busy !== null}
                                onClick={() => void choose(plugin, true)}
                              >
                                Add
                              </button>
                            )}
                          </div>
                        </div>
                      );
                    })}
                  </div>
                ) : (
                  <p className="customizations-muted">
                    {query ? "No plugins match." : "Every approved plugin is already yours."}
                  </p>
                )}
              </section>
            )}
          </>
        )}
      </div>
    </main>
  );
}

function PluginIcon({ plugin }: { plugin: Plugin }) {
  return (
    <span className="plugin-icon" aria-hidden>
      {(plugin.display_name ?? plugin.name).slice(0, 1).toUpperCase()}
    </span>
  );
}
