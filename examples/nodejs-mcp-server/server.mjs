/**
 * Example: Node.js MCP server that extends chisel.
 *
 * This server exposes a domain-specific tool (`scaffold_component`) that
 * builds file content and delegates the actual write to a running chisel
 * instance via the MCP SDK client. The LLM only sees one server.
 *
 * Prerequisites:
 *   npm install
 *   MCP_APP_SECRET=mysecret chisel --root /path/to/project   # in another terminal
 *
 * Run:
 *   CHISEL_SECRET=mysecret CHISEL_ROOT=/path/to/project node server.mjs
 */

import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import { Client } from "@modelcontextprotocol/sdk/client/index.js";
import { StreamableHTTPClientTransport } from "@modelcontextprotocol/sdk/client/streamableHttp.js";
import { z } from "zod";

// ── chisel client setup ────────────────────────────────────────────────────

const CHISEL_URL   = process.env.CHISEL_URL    ?? "http://127.0.0.1:3000/mcp";
const CHISEL_SECRET = process.env.CHISEL_SECRET;
const CHISEL_ROOT   = process.env.CHISEL_ROOT  ?? "/data";

if (!CHISEL_SECRET) {
  console.error("CHISEL_SECRET env var is required");
  process.exit(1);
}

const chisel = new Client({ name: "nodejs-mcp-server-example", version: "1.0.0" });

await chisel.connect(
  new StreamableHTTPClientTransport(new URL(CHISEL_URL), {
    requestInit: {
      headers: { Authorization: `Bearer ${CHISEL_SECRET}` },
    },
  })
);

// Thin helpers — each delegates directly to chisel's MCP tools.
const chiselWriteFile = (path, content) =>
  chisel.callTool({ name: "write_file", arguments: { path, content } });

const chiselPatchApply = (path, patch) =>
  chisel.callTool({ name: "patch_apply", arguments: { path, patch } });

const chiselShellExec = (command, args) =>
  chisel.callTool({ name: "shell_exec", arguments: { command, args } });

// ── your MCP server ────────────────────────────────────────────────────────

const server = new McpServer({
  name: "my-project-server",
  version: "1.0.0",
});

/**
 * scaffold_component
 * Creates a new React component file with TypeScript boilerplate.
 * File writing is delegated to chisel — path confinement and atomicity
 * are enforced server-side at no extra cost.
 */
server.tool(
  "scaffold_component",
  "Scaffold a new React TypeScript component file",
  {
    name:      z.string().describe("PascalCase component name, e.g. UserCard"),
    directory: z.string().describe("Absolute path to the target directory"),
  },
  async ({ name, directory }) => {
    const path    = `${CHISEL_ROOT}/${directory}/${name}.tsx`;
    const content = `import React from "react";

interface ${name}Props {}

export function ${name}({}: ${name}Props) {
  return (
    <div className="${name.toLowerCase()}">
      {/* TODO */}
    </div>
  );
}

export default ${name};
`;
    const result = await chiselWriteFile(path, content);
    return {
      content: [{ type: "text", text: `Created ${path}\n${result.content[0].text}` }],
    };
  }
);

/**
 * run_tests
 * Runs the project's test suite and returns the output.
 * Uses chisel's shell_exec so path args are confined to the project root.
 */
server.tool(
  "run_tests",
  "Run the project test suite using npm test",
  { filter: z.string().optional().describe("Optional test name filter") },
  async ({ filter }) => {
    const args = ["test", "--", "--passWithNoTests"];
    if (filter) args.push("--testNamePattern", filter);

    const result = await chiselShellExec("npm", args);
    const out    = result.content[0].text;
    return { content: [{ type: "text", text: out }] };
  }
);

// ── start ──────────────────────────────────────────────────────────────────

const transport = new StdioServerTransport();
await server.connect(transport);
