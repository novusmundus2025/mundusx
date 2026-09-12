#!/usr/bin/env node
// Serves the static site built by build-public-docs-site.mjs on Railway.
// Zero dependencies so it needs nothing beyond the Node runtime Railpack already installs.
import { createServer } from "node:http";
import { createReadStream, statSync } from "node:fs";
import { extname, join, normalize, resolve } from "node:path";

const root = resolve(process.argv[2] || "dist/public-docs-site");
const port = Number(process.env.PORT) || 3002;
const host = "0.0.0.0";

const MIME_TYPES = {
  ".html": "text/html; charset=utf-8",
  ".js": "text/javascript; charset=utf-8",
  ".mjs": "text/javascript; charset=utf-8",
  ".css": "text/css; charset=utf-8",
  ".json": "application/json; charset=utf-8",
  ".svg": "image/svg+xml",
  ".png": "image/png",
  ".jpg": "image/jpeg",
  ".jpeg": "image/jpeg",
  ".ico": "image/x-icon",
  ".txt": "text/plain; charset=utf-8",
  ".sh": "text/x-shellscript; charset=utf-8",
  ".ps1": "text/plain; charset=utf-8",
};

function resolveFile(urlPath) {
  const safePath = normalize(urlPath).replace(/^(\.\.[/\\])+/, "");
  let filePath = join(root, safePath);

  try {
    if (statSync(filePath).isDirectory()) {
      filePath = join(filePath, "index.html");
    }
  } catch {
    // Not found as-is; fall through to the not-found handling below.
  }

  try {
    const stat = statSync(filePath);
    if (stat.isFile()) return filePath;
  } catch {
    // Fall through.
  }

  return null;
}

const server = createServer((req, res) => {
  const urlPath = decodeURIComponent((req.url || "/").split("?")[0]);
  const filePath = resolveFile(urlPath) || join(root, "404.html");

  let stat;
  try {
    stat = statSync(filePath);
  } catch {
    res.writeHead(404, { "content-type": "text/plain; charset=utf-8" });
    res.end("404 Not Found");
    return;
  }

  const contentType = MIME_TYPES[extname(filePath)] || "application/octet-stream";
  res.writeHead(200, {
    "content-type": contentType,
    "content-length": stat.size,
  });
  createReadStream(filePath).pipe(res);
});

server.listen(port, host, () => {
  console.log(`Serving ${root} on http://${host}:${port}`);
});
