// A fake muster view: serves the real shell and assets the way the crate does,
// under a capability path, with a scriptable event stream and log route.
import { readFileSync } from "node:fs";
import { createServer } from "node:http";
import path from "node:path";
import { fileURLToPath } from "node:url";

const HERE = path.dirname(fileURLToPath(import.meta.url));
const ASSETS = path.resolve(HERE, "../../assets");
const WORKSPACE_MANIFEST = path.resolve(HERE, "../../../Cargo.toml");
const TOKEN = "faketoken";
const BASE = `/v/${TOKEN}/`;
const CSP =
  "default-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; connect-src 'self'";
const MEDIA_TYPES = {
  ".js": "text/javascript; charset=utf-8",
  ".mjs": "text/javascript; charset=utf-8",
  ".css": "text/css; charset=utf-8",
  ".svg": "image/svg+xml",
  ".woff2": "font/woff2",
  ".txt": "text/plain; charset=utf-8",
};

/** The version the crate stamps into its asset root. */
function crateVersion() {
  const manifest = readFileSync(WORKSPACE_MANIFEST, "utf8");
  const section = manifest.split(/^\[workspace\.package\]/m)[1] ?? "";
  const match = section.match(/^version\s*=\s*"([^"]+)"/m);
  if (!match) throw new Error("no workspace version found");
  return match[1];
}

/** One SSE frame for `message`, shaped as the crate writes it. */
export function frame(message) {
  return `event: ${message.kind}\nid: ${message.seq}\ndata: ${JSON.stringify(message)}\n\n`;
}

/**
 * Starts the fake on a free port. `script` is what each new stream sends on
 * connect; `lines` is what the log route answers.
 */
export async function startFake() {
  const version = crateVersion();
  const assetRoot = `assets/v${version}`;
  const shell = readFileSync(path.join(ASSETS, "shell.html"), "utf8").replaceAll(
    "{asset-root}",
    assetRoot,
  );
  const fake = {
    script: [],
    lines: [],
    streams: 0,
    open: new Set(),
    unserved: [],
    send(message) {
      for (const response of fake.open) response.write(frame(message));
    },
    drop() {
      for (const response of fake.open) response.destroy();
      fake.open.clear();
    },
  };

  const server = createServer((request, response) => {
    const url = new URL(request.url, "http://fake");
    const headers = { "Content-Security-Policy": CSP, "Cache-Control": "no-store" };
    if (!url.pathname.startsWith(BASE)) return notFound(response, headers, request.url);
    const rest = url.pathname.slice(BASE.length);
    if (rest === "") {
      response.writeHead(200, { ...headers, "Content-Type": "text/html; charset=utf-8" });
      return response.end(shell);
    }
    if (rest === "events") {
      fake.streams += 1;
      response.writeHead(200, {
        ...headers,
        "Content-Type": "text/event-stream",
        Connection: "keep-alive",
      });
      fake.open.add(response);
      response.on("close", () => fake.open.delete(response));
      for (const message of fake.script) response.write(frame(message));
      return;
    }
    if (rest === "api/log") {
      response.writeHead(200, { ...headers, "Content-Type": "application/json" });
      return response.end(JSON.stringify({ lines: fake.lines }));
    }
    if (rest.startsWith(`${assetRoot}/`)) {
      const file = path.resolve(ASSETS, rest.slice(assetRoot.length + 1));
      if (!file.startsWith(ASSETS + path.sep)) return notFound(response, headers, request.url);
      try {
        const body = readFileSync(file);
        const type = MEDIA_TYPES[path.extname(file)] ?? "application/octet-stream";
        response.writeHead(200, { ...headers, "Content-Type": type });
        return response.end(body);
      } catch {
        return notFound(response, headers, request.url);
      }
    }
    return notFound(response, headers, request.url);
  });

  /** Answers 404 and remembers the path, so a test can see what was asked in vain. */
  function notFound(response, headers, requested) {
    fake.unserved.push(requested);
    response.writeHead(404, headers);
    response.end();
  }

  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  fake.origin = `http://127.0.0.1:${server.address().port}`;
  fake.url = `${fake.origin}${BASE}`;
  fake.close = () => {
    fake.drop();
    server.closeAllConnections();
    return new Promise((resolve) => server.close(resolve));
  };
  return fake;
}

/** A process as the floor sends it, with `fields` over a running default. */
export function face(name, fields = {}) {
  return {
    name,
    next: { verb: "attach", command: `muster -p demo attach ${name}` },
    icon: "terminal",
    room: "running",
    mark: null,
    state: "running",
    ports: [],
    started: Math.floor(Date.now() / 1000) - 90,
    cpu: null,
    memory: null,
    restarts: 0,
    ended: null,
    attached: 0,
    speaking: false,
    ...fields,
  };
}

/** A project floor holding `processes`, with `fields` over a readable default. */
export function floor(name, processes, fields = {}) {
  return { name, path: `/work/${name}`, icon: "terminal", processes, unreadable: false, ...fields };
}

/** A snapshot message numbered `seq`. */
export function snapshot(seq, projects, extra = {}) {
  return {
    kind: "snapshot",
    seq,
    health: "running",
    here: projects[0]?.name ?? null,
    projects,
    notes: {},
    ...extra,
  };
}
