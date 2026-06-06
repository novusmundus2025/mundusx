#!/usr/bin/env node

import fs from "node:fs";
import path from "node:path";

const repoRoot = path.resolve(path.dirname(new URL(import.meta.url).pathname), "..");
const outputDir = path.resolve(process.argv[2] ?? path.join(repoRoot, "dist/public-docs-site"));
const repoReleaseBaseUrl = "https://github.com/mundusx/mundusx/releases/latest/download";
const repoReleaseNotesUrl = "https://github.com/mundusx/mundusx/releases/latest";
const installScriptContents = fs.readFileSync(path.join(repoRoot, "install.sh"), "utf8");

const pages = [
  {
    slug: "",
    title: "NovusX Docs",
    sourcePath: null,
    intro:
      "Public-facing documentation for install, device identity, and release flow previews in the MundusX repo.",
  },
  {
    slug: "install",
    title: "Install NovusX",
    sourcePath: path.join(repoRoot, "docs/install-page.md"),
    intro: "Canonical install flow and command for the current localhost-first release path.",
  },
  {
    slug: "device-identity",
    title: "Device Identity Lifecycle",
    sourcePath: path.join(repoRoot, "docs/device-identity-lifecycle.md"),
    intro: "How contributor identity is stored, reused, and reset across the NovusX lifecycle.",
  },
  {
    slug: "release",
    title: "Install And Release Strategy",
    sourcePath: path.join(repoRoot, "docs/install-strategy.md"),
    intro: "Release-hosting and distribution notes that support the public install surface.",
  },
];

fs.rmSync(outputDir, { recursive: true, force: true });
fs.mkdirSync(outputDir, { recursive: true });

writeDocsVariant({
  outputBaseDir: outputDir,
  eyebrow: "Public Repo Docs",
  homeIntro:
    "Public-facing documentation for install, device identity, and release flow previews in the MundusX repo.",
});
writeDocsVariant({
  outputBaseDir: path.join(outputDir, "public", "docs"),
  eyebrow: "Public Endpoint Mirror",
  homeIntro:
    "GitHub Pages mirror of the public docs shape so install, device identity, and release pages can be reviewed before the final domain is wired up.",
});
writePublicInstallSurface();
writePublicReleaseSurface();

fs.writeFileSync(path.join(outputDir, ".nojekyll"), "\n");

function writeDocsVariant({ outputBaseDir, eyebrow, homeIntro }) {
  for (const page of pages) {
    const html = renderPage(page, { eyebrow, homeIntro });
    const pageDir = page.slug ? path.join(outputBaseDir, page.slug) : outputBaseDir;
    fs.mkdirSync(pageDir, { recursive: true });
    fs.writeFileSync(path.join(pageDir, "index.html"), html);
  }
}

function writePublicInstallSurface() {
  const publicDir = path.join(outputDir, "public");
  const installDir = path.join(publicDir, "install");
  fs.mkdirSync(installDir, { recursive: true });

  const manifest = {
    kind: "install-manifest",
    product_name: "NovusX",
    audience: "public",
    install_script_href: "./install.sh",
    release_base_url: repoReleaseBaseUrl,
    release_notes_url: repoReleaseNotesUrl,
    docs_home_href: "./docs/",
    install_docs_href: "./docs/install/",
    checksum_hint:
      "The hosted install script uses the signed GitHub release artifacts and verifies checksums when the matching .sha256 file is published.",
  };

  fs.writeFileSync(path.join(publicDir, "install.sh"), installScriptContents);
  fs.writeFileSync(path.join(publicDir, "install.json"), JSON.stringify(manifest, null, 2) + "\n");
  fs.writeFileSync(path.join(installDir, "index.html"), renderPublicInstallPage());
}

function writePublicReleaseSurface() {
  const publicDir = path.join(outputDir, "public");
  const releaseDir = path.join(publicDir, "release");
  fs.mkdirSync(releaseDir, { recursive: true });

  const manifest = {
    kind: "release-channel",
    product_name: "NovusX",
    audience: "public",
    release_base_url: repoReleaseBaseUrl,
    release_notes_url: repoReleaseNotesUrl,
    install_surface_href: "./install/",
    install_manifest_href: "./install.json",
    docs_release_href: "./docs/release/",
    artifact_name_pattern: "opengpu-<target>",
    checksum_suffix: ".sha256",
    signed_manifest_name: "release-manifest.json",
    distribution_summary:
      "GitHub Releases remains the current public artifact host while the Pages mirror explains the release channel, checksums, and install entrypoints.",
  };

  fs.writeFileSync(path.join(publicDir, "release.json"), JSON.stringify(manifest, null, 2) + "\n");
  fs.writeFileSync(path.join(releaseDir, "index.html"), renderPublicReleasePage());
}

function renderPage(page, options) {
  const { eyebrow, homeIntro } = options;
  const nav = pages
    .map((entry) => {
      const href = relativeHref(page.slug, entry.slug);
      const active = entry.slug === page.slug ? ' aria-current="page"' : "";
      return `<a href="${href}"${active}>${escapeHtml(entry.title)}</a>`;
    })
    .join("");

  const body = page.sourcePath
    ? renderMarkdown(page, fs.readFileSync(page.sourcePath, "utf8"))
    : renderHome(homeIntro);

  return `<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <title>${escapeHtml(page.title)}</title>
    <style>
      :root {
        color-scheme: light;
        --bg: #f6f3ea;
        --panel: #fffdf8;
        --text: #1e1d1a;
        --muted: #5d584f;
        --accent: #0b6bcb;
        --border: #d9d2c3;
        --shadow: 0 18px 48px rgba(51, 42, 20, 0.08);
      }
      * { box-sizing: border-box; }
      body {
        margin: 0;
        font-family: "Iowan Old Style", "Palatino Linotype", serif;
        background: linear-gradient(180deg, #f2ece0 0%, var(--bg) 22%, #fcfbf8 100%);
        color: var(--text);
      }
      a { color: var(--accent); }
      header {
        padding: 3rem 1.5rem 1rem;
      }
      .shell {
        width: min(1100px, calc(100% - 2rem));
        margin: 0 auto 4rem;
      }
      .hero {
        background: radial-gradient(circle at top left, rgba(11, 107, 203, 0.12), transparent 35%), var(--panel);
        border: 1px solid var(--border);
        border-radius: 28px;
        box-shadow: var(--shadow);
        padding: 2rem;
      }
      .eyebrow {
        display: inline-block;
        margin: 0 0 1rem;
        padding: 0.3rem 0.7rem;
        border-radius: 999px;
        background: #efe7d6;
        color: var(--muted);
        font: 600 0.85rem/1.2 "Helvetica Neue", Arial, sans-serif;
        letter-spacing: 0.04em;
        text-transform: uppercase;
      }
      h1 {
        margin: 0;
        font-size: clamp(2.4rem, 4vw, 4.4rem);
        line-height: 0.98;
      }
      .hero p {
        max-width: 48rem;
        font-size: 1.1rem;
        line-height: 1.65;
        color: var(--muted);
      }
      nav {
        display: flex;
        flex-wrap: wrap;
        gap: 0.75rem;
        margin-top: 1.5rem;
      }
      nav a {
        text-decoration: none;
        color: var(--text);
        padding: 0.65rem 0.9rem;
        border-radius: 999px;
        border: 1px solid var(--border);
        background: rgba(255, 255, 255, 0.75);
        font: 600 0.95rem/1.2 "Helvetica Neue", Arial, sans-serif;
      }
      nav a[aria-current="page"] {
        background: var(--accent);
        border-color: var(--accent);
        color: white;
      }
      main {
        margin-top: 1.5rem;
        background: var(--panel);
        border: 1px solid var(--border);
        border-radius: 28px;
        box-shadow: var(--shadow);
        padding: 2rem;
      }
      .cards {
        display: grid;
        grid-template-columns: repeat(auto-fit, minmax(220px, 1fr));
        gap: 1rem;
        margin-top: 2rem;
      }
      .card {
        display: block;
        padding: 1.2rem;
        border-radius: 20px;
        text-decoration: none;
        background: #fff;
        border: 1px solid var(--border);
        color: inherit;
      }
      .card h2 {
        margin-top: 0;
        font-size: 1.2rem;
      }
      article {
        max-width: 52rem;
        margin: 0 auto;
      }
      article h1:first-child {
        font-size: clamp(2rem, 3vw, 3rem);
      }
      article h2, article h3 {
        margin-top: 2rem;
      }
      article p, article li {
        font-size: 1.05rem;
        line-height: 1.75;
      }
      article code {
        background: #f3eee4;
        border-radius: 6px;
        padding: 0.12rem 0.35rem;
        font-size: 0.95em;
      }
      pre {
        overflow-x: auto;
        padding: 1rem;
        border-radius: 18px;
        background: #1f2430;
        color: #f4f1eb;
      }
      pre code {
        background: none;
        padding: 0;
      }
      ul, ol {
        padding-left: 1.3rem;
      }
      footer {
        max-width: 52rem;
        margin: 2rem auto 0;
        color: var(--muted);
        font: 0.95rem/1.6 "Helvetica Neue", Arial, sans-serif;
      }
      @media (max-width: 640px) {
        .hero, main {
          padding: 1.4rem;
          border-radius: 22px;
        }
      }
    </style>
  </head>
  <body>
    <div class="shell">
      <header>
        <div class="hero">
          <span class="eyebrow">${escapeHtml(eyebrow)}</span>
          <h1>${escapeHtml(page.title)}</h1>
          <p>${escapeHtml(page.intro)}</p>
          <nav>${nav}</nav>
        </div>
      </header>
      <main>${body}</main>
      <footer>
        Built from the tracked markdown in this repository so the public docs site stays reviewable in pull requests.
      </footer>
    </div>
  </body>
</html>`;
}

function renderHome(homeIntro) {
  return `
    <article>
      <p>${escapeHtml(homeIntro)}</p>
      <div class="cards">
        ${pages
          .filter((entry) => entry.slug)
          .map(
            (entry) => `<a class="card" href="${relativeHref("", entry.slug)}">
              <h2>${escapeHtml(entry.title)}</h2>
              <p>${escapeHtml(entry.intro)}</p>
            </a>`,
          )
          .join("")}
      </div>
    </article>`;
}

function renderMarkdown(page, markdown) {
  const lines = markdown.replace(/\r\n/g, "\n").split("\n");
  const html = [];
  let paragraph = [];
  let listType = null;
  let listItems = [];
  let codeFence = null;
  let codeLines = [];

  const flushParagraph = () => {
    if (paragraph.length === 0) return;
    html.push(`<p>${renderInline(paragraph.join(" "))}</p>`);
    paragraph = [];
  };

  const flushList = () => {
    if (!listType || listItems.length === 0) return;
    html.push(`<${listType}>${listItems.map((item) => `<li>${renderInline(item)}</li>`).join("")}</${listType}>`);
    listType = null;
    listItems = [];
  };

  const flushCodeFence = () => {
    if (codeFence === null) return;
    const language = codeFence ? ` class="language-${escapeHtml(codeFence)}"` : "";
    html.push(`<pre><code${language}>${escapeHtml(codeLines.join("\n"))}</code></pre>`);
    codeFence = null;
    codeLines = [];
  };

  for (const line of lines) {
    const fenceMatch = line.match(/^```(.*)$/);
    if (fenceMatch) {
      if (codeFence !== null) {
        flushCodeFence();
      } else {
        flushParagraph();
        flushList();
        codeFence = fenceMatch[1].trim();
      }
      continue;
    }

    if (codeFence !== null) {
      codeLines.push(line);
      continue;
    }

    const headingMatch = line.match(/^(#{1,3})\s+(.*)$/);
    if (headingMatch) {
      flushParagraph();
      flushList();
      const level = headingMatch[1].length;
      html.push(`<h${level}>${renderInline(headingMatch[2])}</h${level}>`);
      continue;
    }

    const listMatch = line.match(/^(\s*)([-*]|\d+\.)\s+(.*)$/);
    if (listMatch) {
      flushParagraph();
      const nextType = /\d+\./.test(listMatch[2]) ? "ol" : "ul";
      if (listType && listType !== nextType) {
        flushList();
      }
      listType = nextType;
      listItems.push(listMatch[3]);
      continue;
    }

    if (line.trim() === "") {
      flushParagraph();
      flushList();
      continue;
    }

    paragraph.push(line.trim());
  }

  flushParagraph();
  flushList();
  flushCodeFence();

  const summaryBlocks = [];
  if (page.slug === "install") {
    summaryBlocks.push(
      `<section><h2>Canonical command</h2><pre><code>RELEASE_BASE_URL=http://127.0.0.1:8788/releases/latest/download bash install.sh</code></pre></section>`,
    );
  }
  if (page.slug === "device-identity") {
    summaryBlocks.push(
      `<section><h2>Current stance</h2><p>NovusX should prefer non-exportable device keys so the CLI, agent, and worker can request signatures without reading raw private-key bytes.</p></section>`,
    );
  }

  return `<article>${summaryBlocks.join("")}${html.join("\n")}</article>`;
}

function renderInline(text) {
  let rendered = escapeHtml(text);
  rendered = rendered.replace(/`([^`]+)`/g, "<code>$1</code>");
  rendered = rendered.replace(/\[([^\]]+)\]\(([^)]+)\)/g, '<a href="$2">$1</a>');
  return rendered;
}

function escapeHtml(text) {
  return text
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;");
}

function relativeHref(fromSlug, toSlug) {
  if (fromSlug === toSlug) {
    return fromSlug === "" ? "./" : "../";
  }
  if (fromSlug === "") {
    return toSlug === "" ? "./" : `./${toSlug}/`;
  }
  if (toSlug === "") {
    return "../";
  }
  return `../${toSlug}/`;
}

function renderPublicInstallPage() {
  return `<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <title>Install NovusX</title>
    <style>
      :root {
        color-scheme: light;
        --bg: #f4efe4;
        --panel: #fffdf8;
        --text: #1c1c19;
        --muted: #5e584d;
        --accent: #0b6bcb;
        --border: #d9d2c3;
        --shadow: 0 20px 56px rgba(55, 42, 20, 0.1);
      }
      * { box-sizing: border-box; }
      body {
        margin: 0;
        font-family: "Iowan Old Style", "Palatino Linotype", serif;
        background:
          radial-gradient(circle at top left, rgba(11, 107, 203, 0.15), transparent 30%),
          linear-gradient(180deg, #efe7d6 0%, var(--bg) 28%, #fbfaf7 100%);
        color: var(--text);
      }
      a { color: var(--accent); }
      .shell {
        width: min(1040px, calc(100% - 2rem));
        margin: 0 auto;
        padding: 2.5rem 0 4rem;
      }
      .hero, .panel {
        background: var(--panel);
        border: 1px solid var(--border);
        border-radius: 28px;
        box-shadow: var(--shadow);
      }
      .hero {
        padding: 2rem;
      }
      .eyebrow {
        display: inline-block;
        margin-bottom: 1rem;
        padding: 0.3rem 0.7rem;
        border-radius: 999px;
        background: #efe7d6;
        color: var(--muted);
        font: 600 0.84rem/1.2 "Helvetica Neue", Arial, sans-serif;
        letter-spacing: 0.05em;
        text-transform: uppercase;
      }
      h1 {
        margin: 0;
        font-size: clamp(2.7rem, 5vw, 4.6rem);
        line-height: 0.98;
      }
      .lead {
        max-width: 46rem;
        font-size: 1.12rem;
        line-height: 1.7;
        color: var(--muted);
      }
      .actions {
        display: flex;
        flex-wrap: wrap;
        gap: 0.75rem;
        margin-top: 1.5rem;
      }
      .actions a {
        text-decoration: none;
        font: 600 0.95rem/1.2 "Helvetica Neue", Arial, sans-serif;
        padding: 0.75rem 1rem;
        border-radius: 999px;
        border: 1px solid var(--border);
        color: var(--text);
        background: rgba(255, 255, 255, 0.8);
      }
      .actions a.primary {
        background: var(--accent);
        border-color: var(--accent);
        color: white;
      }
      .panel {
        margin-top: 1.4rem;
        padding: 1.8rem;
      }
      .panel h2 {
        margin-top: 0;
      }
      .command {
        overflow-x: auto;
        padding: 1rem 1.1rem;
        border-radius: 20px;
        background: #1f2430;
        color: #f4f1eb;
        font-size: 1rem;
        line-height: 1.6;
      }
      .manifest {
        margin-top: 1rem;
        color: var(--muted);
        font: 0.95rem/1.6 "Helvetica Neue", Arial, sans-serif;
      }
      .grid {
        display: grid;
        grid-template-columns: repeat(auto-fit, minmax(220px, 1fr));
        gap: 1rem;
      }
      .card {
        padding: 1rem;
        border-radius: 20px;
        background: #fff;
        border: 1px solid var(--border);
      }
      .card h3 {
        margin-top: 0;
      }
      code {
        background: #f3eee4;
        border-radius: 6px;
        padding: 0.12rem 0.35rem;
      }
      ul {
        padding-left: 1.2rem;
      }
      @media (max-width: 640px) {
        .hero, .panel {
          border-radius: 22px;
          padding: 1.4rem;
        }
      }
    </style>
  </head>
  <body>
    <div class="shell">
      <section class="hero">
        <span class="eyebrow">Public Install Endpoint</span>
        <h1>Install NovusX on your Mac</h1>
        <p class="lead">This Pages-backed install surface mirrors the future public endpoint shape. It hosts the reviewed installer script from this repo and points that script at the latest signed GitHub release artifacts.</p>
        <div class="actions">
          <a class="primary" href="../install.sh">Download install.sh</a>
          <a href="../docs/install/">Install docs</a>
          <a href="../docs/release/">Release notes and distribution</a>
        </div>
      </section>

      <section class="panel">
        <h2>One command</h2>
        <div class="command" id="install-command">Fetching ../install.json</div>
        <p class="manifest" id="manifest-state">The public install shell loads its command and release source from <code>../install.json</code>.</p>
      </section>

      <section class="panel">
        <h2>What this endpoint guarantees</h2>
        <div class="grid">
          <div class="card">
            <h3>Repo-owned script</h3>
            <p>The same checked-in <code>install.sh</code> from this repo is published at <code>/public/install.sh</code>.</p>
          </div>
          <div class="card">
            <h3>Signed release source</h3>
            <p>The installer defaults to the latest GitHub release download set and verifies the matching checksum file when it is published.</p>
          </div>
          <div class="card">
            <h3>Reviewable copy</h3>
            <p>The broader docs mirror stays available under <code>/public/docs</code> so install wording and release notes can be reviewed together.</p>
          </div>
        </div>
      </section>

      <section class="panel">
        <h2>Before you run it</h2>
        <ul>
          <li>Apple Silicon Macs remain the primary release channel today.</li>
          <li>The installer places <code>opengpu</code> into <code>$HOME/.local/bin</code> by default.</li>
          <li>After install, use <code>opengpu cap</code> before <code>opengpu start</code> so the node budget is explicit.</li>
        </ul>
      </section>
    </div>
    <script>
      const manifestUrl = new URL("../install.json", window.location.href);
      const commandNode = document.getElementById("install-command");
      const stateNode = document.getElementById("manifest-state");

      async function loadInstallManifest() {
        const response = await fetch(manifestUrl, { headers: { Accept: "application/json" } });
        if (!response.ok) {
          throw new Error("manifest request failed with " + response.status);
        }
        const manifest = await response.json();
        const installScriptUrl = new URL(manifest.install_script_href, manifestUrl).href;
        const command = "curl -fsSL " + installScriptUrl + " | bash";
        commandNode.textContent = command;
        stateNode.textContent =
          "Manifest loaded from ../install.json • GitHub release source: " + manifest.release_base_url;
      }

      loadInstallManifest().catch((error) => {
        commandNode.textContent = "Failed to load install manifest";
        stateNode.textContent = "Unable to load ../install.json: " + error.message;
      });
    </script>
  </body>
</html>`;
}

function renderPublicReleasePage() {
  return `<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <title>Public Release Mirror</title>
    <style>
      :root {
        color-scheme: light;
        --bg: #eef5f4;
        --panel: #fbfefd;
        --text: #162321;
        --muted: #526865;
        --accent: #0d8b78;
        --border: #c7ddd8;
        --shadow: 0 18px 56px rgba(18, 55, 48, 0.1);
      }
      * { box-sizing: border-box; }
      body {
        margin: 0;
        font-family: "Iowan Old Style", "Palatino Linotype", serif;
        background:
          radial-gradient(circle at top right, rgba(13, 139, 120, 0.14), transparent 28%),
          linear-gradient(180deg, #e7f1ee 0%, var(--bg) 30%, #f9fcfb 100%);
        color: var(--text);
      }
      a { color: var(--accent); }
      .shell {
        width: min(1080px, calc(100% - 2rem));
        margin: 0 auto;
        padding: 2.5rem 0 4rem;
      }
      .hero, .panel {
        background: var(--panel);
        border: 1px solid var(--border);
        border-radius: 28px;
        box-shadow: var(--shadow);
      }
      .hero {
        padding: 2rem;
      }
      .eyebrow {
        display: inline-block;
        margin-bottom: 1rem;
        padding: 0.3rem 0.7rem;
        border-radius: 999px;
        background: #ddeeea;
        color: var(--muted);
        font: 600 0.84rem/1.2 "Helvetica Neue", Arial, sans-serif;
        letter-spacing: 0.05em;
        text-transform: uppercase;
      }
      h1 {
        margin: 0;
        font-size: clamp(2.6rem, 5vw, 4.4rem);
        line-height: 0.98;
      }
      .lead {
        max-width: 48rem;
        font-size: 1.1rem;
        line-height: 1.7;
        color: var(--muted);
      }
      .actions {
        display: flex;
        flex-wrap: wrap;
        gap: 0.75rem;
        margin-top: 1.5rem;
      }
      .actions a {
        text-decoration: none;
        font: 600 0.95rem/1.2 "Helvetica Neue", Arial, sans-serif;
        padding: 0.75rem 1rem;
        border-radius: 999px;
        border: 1px solid var(--border);
        color: var(--text);
        background: rgba(255, 255, 255, 0.8);
      }
      .actions a.primary {
        background: var(--accent);
        border-color: var(--accent);
        color: white;
      }
      .panel {
        margin-top: 1.4rem;
        padding: 1.8rem;
      }
      .panel h2 {
        margin-top: 0;
      }
      .manifest {
        overflow-x: auto;
        padding: 1rem 1.1rem;
        border-radius: 20px;
        background: #1e2a2a;
        color: #ecf6f4;
        font-size: 1rem;
        line-height: 1.6;
      }
      .status {
        margin-top: 1rem;
        color: var(--muted);
        font: 0.95rem/1.6 "Helvetica Neue", Arial, sans-serif;
      }
      .grid {
        display: grid;
        grid-template-columns: repeat(auto-fit, minmax(220px, 1fr));
        gap: 1rem;
      }
      .card {
        padding: 1rem;
        border-radius: 20px;
        background: #fff;
        border: 1px solid var(--border);
      }
      .card h3 {
        margin-top: 0;
      }
      code {
        background: #e7f1ee;
        border-radius: 6px;
        padding: 0.12rem 0.35rem;
      }
      ul {
        padding-left: 1.2rem;
      }
      @media (max-width: 640px) {
        .hero, .panel {
          border-radius: 22px;
          padding: 1.4rem;
        }
      }
    </style>
  </head>
  <body>
    <div class="shell">
      <section class="hero">
        <span class="eyebrow">Public Release Mirror</span>
        <h1>Review the live release channel before the final domain exists.</h1>
        <p class="lead">This Pages-backed release surface keeps the current GitHub-hosted distribution flow legible in pull requests. It points at the signed release artifact channel, the checked-in installer surface, and the release notes users should trust today.</p>
        <div class="actions">
          <a class="primary" href="https://github.com/mundusx/mundusx/releases/latest">Latest release notes</a>
          <a href="../install/">Install endpoint</a>
          <a href="../docs/release/">Release strategy docs</a>
        </div>
      </section>

      <section class="panel">
        <h2>Release channel manifest</h2>
        <div class="manifest" id="release-summary">Fetching ../release.json</div>
        <p class="status" id="release-state">This mirror loads its release-channel metadata from <code>../release.json</code>.</p>
      </section>

      <section class="panel">
        <h2>What this release surface covers</h2>
        <div class="grid">
          <div class="card">
            <h3>Artifact host</h3>
            <p>GitHub Releases stays the current public binary host, with the latest download set rooted at <code>/releases/latest/download</code>.</p>
          </div>
          <div class="card">
            <h3>Verification path</h3>
            <p>Each published asset is expected to ship with a checksum and a signed <code>release-manifest.json</code>.</p>
          </div>
          <div class="card">
            <h3>Install hand-off</h3>
            <p>The public install surface and checked-in <code>install.sh</code> remain the supported way to consume that release channel.</p>
          </div>
        </div>
      </section>

      <section class="panel">
        <h2>Current release posture</h2>
        <ul>
          <li>Apple Silicon macOS remains the primary release lane today.</li>
          <li>Package-manager distribution can layer on later without changing the signed GitHub release source.</li>
          <li>The Pages mirror is for reviewability; the actual binary artifacts still come from GitHub Releases.</li>
        </ul>
      </section>
    </div>
    <script>
      const manifestUrl = new URL("../release.json", window.location.href);
      const summaryNode = document.getElementById("release-summary");
      const stateNode = document.getElementById("release-state");

      async function loadReleaseManifest() {
        const response = await fetch(manifestUrl, { headers: { Accept: "application/json" } });
        if (!response.ok) {
          throw new Error("release manifest request failed with " + response.status);
        }
        const manifest = await response.json();
        summaryNode.textContent =
          "Latest artifacts: " +
          manifest.release_base_url +
          " | Notes: " +
          manifest.release_notes_url +
          " | Pattern: " +
          manifest.artifact_name_pattern;
        stateNode.textContent =
          "Manifest loaded from ../release.json • Signed manifest: " + manifest.signed_manifest_name;
      }

      loadReleaseManifest().catch((error) => {
        summaryNode.textContent = "Failed to load release channel manifest";
        stateNode.textContent = "Unable to load ../release.json: " + error.message;
      });
    </script>
  </body>
</html>`;
}
