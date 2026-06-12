import { useState } from "react";
import { fireEvent, render, screen } from "@testing-library/react";
import { McpServersEditor } from "@/components/capabilities/mcp-servers-editor";
import { SkillsEditor } from "@/components/capabilities/skills-editor";
import { FilesEditor } from "@/components/capabilities/files-editor";
import {
  definitionToFormState,
  entriesToFiles,
  entriesToMcpServers,
  entriesToSkills,
  formStateToDefinition,
  mcpServersToEntries,
  newFileEntry,
  newMcpServerEntry,
  newSkillEntry,
  type FileEntry,
  type McpServerEntry,
  type SkillEntry,
} from "@/components/capabilities/declarative-form-types";
import type { DeclarativeCapabilityDefinition } from "@/lib/api/types";

describe("declarative capability form conversions", () => {
  it("serializes an HTTP MCP server with headers and drops empty fields", () => {
    const entry: McpServerEntry = {
      ...newMcpServerEntry(),
      name: "atlassian",
      transportType: "http",
      url: " https://mcp.example.com/v1 ",
      headers: [
        { id: "h1", key: "X-Env", value: "prod" },
        { id: "h2", key: "  ", value: "ignored" },
      ],
      authMode: "oauth",
      oauthProviderId: "atlassian",
      toolDiscovery: false,
    };

    expect(entriesToMcpServers([entry])).toEqual({
      atlassian: {
        type: "http",
        url: "https://mcp.example.com/v1",
        headers: { "X-Env": "prod" },
        auth_mode: "oauth",
        oauth_provider_id: "atlassian",
        tool_discovery: false,
      },
    });
  });

  it("serializes a stdio MCP server with args and env", () => {
    const entry: McpServerEntry = {
      ...newMcpServerEntry(),
      name: "fs",
      transportType: "stdio",
      command: "npx",
      args: "-y\n@modelcontextprotocol/server-filesystem\n",
      env: [{ id: "e1", key: "ROOT", value: "/tmp" }],
    };

    expect(entriesToMcpServers([entry])).toEqual({
      fs: {
        type: "stdio",
        command: "npx",
        args: ["-y", "@modelcontextprotocol/server-filesystem"],
        env: { ROOT: "/tmp" },
      },
    });
  });

  it("omits unnamed MCP servers and returns undefined when none remain", () => {
    expect(entriesToMcpServers([{ ...newMcpServerEntry() }])).toBeUndefined();
  });

  it("round-trips MCP servers through parse and serialize", () => {
    const servers = {
      remote: { type: "http", url: "https://a.example/mcp", headers: { Authorization: "x" } },
      local: { type: "stdio", command: "run", args: ["--flag"], tool_discovery: false },
    };
    expect(entriesToMcpServers(mcpServersToEntries(servers))).toEqual(servers);
  });

  it("serializes skills and filters blank bundled files", () => {
    const skill: SkillEntry = {
      ...newSkillEntry(),
      name: "research-helper",
      description: "Use for research",
      instructions: "Do the thing",
      files: [
        { id: "a", path: "ref.md", content: "body" },
        { id: "b", path: "  ", content: "skipme" },
      ],
      userInvocable: true,
      disableModelInvocation: true,
    };

    expect(entriesToSkills([skill])).toEqual([
      {
        name: "research-helper",
        description: "Use for research",
        instructions: "Do the thing",
        files: [{ path: "ref.md", content: "body" }],
        user_invocable: true,
        disable_model_invocation: true,
      },
    ]);
  });

  it("serializes files and skips blank paths", () => {
    const files: FileEntry[] = [
      { ...newFileEntry(), path: "/docs/README.md", content: "hi", access: "readwrite" },
      { ...newFileEntry(), path: "   ", content: "skip" },
    ];
    expect(entriesToFiles(files)).toEqual([
      { path: "/docs/README.md", content: "hi", access: "readwrite" },
    ]);
  });

  it("round-trips a full definition through form state", () => {
    const definition: DeclarativeCapabilityDefinition = {
      name: "research_pack",
      display_name: "Research Pack",
      description: "Research helpers",
      category: "Declarative",
      icon: "puzzle",
      system_prompt: "You are helpful.",
      mcp_servers: { remote: { type: "http", url: "https://a.example/mcp" } },
      skills: [
        {
          name: "helper",
          description: "d",
          instructions: "i",
          files: [],
          user_invocable: true,
          disable_model_invocation: false,
        },
      ],
      files: [{ path: "/a.txt", content: "x", access: "readonly" }],
      dependencies: ["session_file_system", "skills"],
      risk_level: "medium",
    };

    expect(formStateToDefinition(definitionToFormState(definition))).toEqual(definition);
  });
});

function McpHarness() {
  const [servers, setServers] = useState<McpServerEntry[]>([]);
  return (
    <div>
      <span data-testid="count">{servers.length}</span>
      <McpServersEditor servers={servers} onChange={setServers} />
    </div>
  );
}

describe("declarative capability editor interactions", () => {
  it("adds and removes an MCP server", () => {
    render(<McpHarness />);
    expect(screen.getByTestId("count").textContent).toBe("0");

    fireEvent.click(screen.getByRole("button", { name: /add mcp server/i }));
    expect(screen.getByTestId("count").textContent).toBe("1");

    fireEvent.click(screen.getByRole("button", { name: /remove server/i }));
    expect(screen.getByTestId("count").textContent).toBe("0");
  });

  it("adds a skill and a file row", () => {
    function SkillHarness() {
      const [skills, setSkills] = useState<SkillEntry[]>([]);
      return (
        <div>
          <span data-testid="skill-count">{skills.length}</span>
          <SkillsEditor skills={skills} onChange={setSkills} />
        </div>
      );
    }
    function FileHarness() {
      const [files, setFiles] = useState<FileEntry[]>([]);
      return (
        <div>
          <span data-testid="file-count">{files.length}</span>
          <FilesEditor files={files} onChange={setFiles} />
        </div>
      );
    }

    const { unmount } = render(<SkillHarness />);
    fireEvent.click(screen.getByRole("button", { name: /add skill/i }));
    expect(screen.getByTestId("skill-count").textContent).toBe("1");
    unmount();

    render(<FileHarness />);
    fireEvent.click(screen.getByRole("button", { name: /add file/i }));
    expect(screen.getByTestId("file-count").textContent).toBe("1");
  });
});
