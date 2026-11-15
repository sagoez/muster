// Leaves a note as an agent would: speaks MCP to `muster mcp` over stdio
// under the client name given, and writes one note through `note_write`.
// Usage: node agent.mjs <client-name> <key> <body>, run in the project's folder
// with the sandbox's environment.
import { spawn } from "node:child_process";
import { createInterface } from "node:readline";

/** The MCP revision the handshake asks for. */
const PROTOCOL_VERSION = "2025-06-18";
/** How long the exchange may take before it is given up. */
const TIMEOUT_MS = 20000;

const [client, key, body] = process.argv.slice(2);
if (!client || !key || !body) {
  console.error("usage: node agent.mjs <client-name> <key> <body>");
  process.exit(2);
}

const server = spawn("muster", ["mcp"], { stdio: ["pipe", "pipe", "inherit"] });
const replies = new Map();
createInterface({ input: server.stdout }).on("line", (line) => {
  let message;
  try {
    message = JSON.parse(line);
  } catch {
    return;
  }
  replies.get(message.id)?.(message);
});

/** Sends request `method` with `params` as id `id`, resolving with its reply. */
function request(id, method, params) {
  return new Promise((resolve) => {
    replies.set(id, resolve);
    server.stdin.write(`${JSON.stringify({ jsonrpc: "2.0", id, method, params })}\n`);
  });
}

const timer = setTimeout(() => {
  console.error("muster mcp did not answer in time");
  server.kill();
  process.exit(1);
}, TIMEOUT_MS);

await request(1, "initialize", {
  protocolVersion: PROTOCOL_VERSION,
  capabilities: {},
  clientInfo: { name: client, version: "1.0.0" },
});
server.stdin.write(`${JSON.stringify({ jsonrpc: "2.0", method: "notifications/initialized" })}\n`);
const reply = await request(2, "tools/call", { name: "note_write", arguments: { key, body } });
clearTimeout(timer);
server.stdin.end();
if (reply.error || reply.result?.isError) {
  console.error(JSON.stringify(reply));
  process.exit(1);
}
