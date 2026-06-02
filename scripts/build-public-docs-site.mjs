#!/usr/bin/env node

import fs from "node:fs";
import path from "node:path";

const repoRoot = path.resolve(path.dirname(new URL(import.meta.url).pathname), "..");
const outputDir = path.resolve(process.argv[2] ?? path.join(repoRoot, "dist/public-docs-site"));

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

for (const page of pages) {
  const html = renderPage(page);
  const pageDir = page.slug ? path.join(outputDir, page.slug) : outputDir;
  fs.mkdirSync(pageDir, { recursive: true });
  fs.writeFileSync(path.join(pageDir, "index.html"), html);
}

fs.writeFileSync(path.join(outputDir, ".nojekyll"), "\n");

function renderPage(page) {
  const nav = pages
    .map((entry) => {
      const href = relativeHref(page.slug, entry.slug);
      const active = entry.slug === page.slug ? ' aria-current="page"' : "";
      return `<a href="${href}"${active}>${escapeHtml(entry.title)}</a>`;
    })
    .join("");

  const body = page.sourcePath
    ? renderMarkdown(page, fs.readFileSync(page.sourcePath, "utf8"))
    : renderHome();

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
          <span class="eyebrow">Public Repo Docs</span>
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

function renderHome() {
  return `
    <article>
      <p>This site is the repo-owned public documentation surface for the current MundusX install and onboarding flow. It is generated directly from the checked-in markdown pages so the deployable site stays aligned with reviewed source docs.</p>
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
