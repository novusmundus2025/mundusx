#!/usr/bin/env node

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const outputDir = path.resolve(process.argv[2] ?? path.join(repoRoot, "dist/public-docs-site"));
const repoReleaseBaseUrl = "https://github.com/mundusx/mundusx/releases/latest/download";
const repoReleaseNotesUrl = "https://github.com/mundusx/mundusx/releases/latest";
const installScriptContents = fs.readFileSync(path.join(repoRoot, "install.sh"), "utf8");
const installPowershellContents = fs.readFileSync(path.join(repoRoot, "install.ps1"), "utf8");

const pages = [
  {
    slug: "",
    title: "MundusX Docs",
    sourcePath: null,
    intro:
      "Public-facing documentation for install, device identity, and release flow previews in the MundusX repo.",
  },
  {
    slug: "install",
    title: "Install MundusX",
    sourcePath: path.join(repoRoot, "docs/install-page.md"),
    intro: "Canonical install flow and command for the current localhost-first release path.",
  },
  {
    slug: "device-identity",
    title: "Device Identity Lifecycle",
    sourcePath: path.join(repoRoot, "docs/device-identity-lifecycle.md"),
    intro: "How contributor identity is stored, reused, and reset across the MundusX lifecycle.",
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
  outputBaseDir: path.join(outputDir, "docs"),
  eyebrow: "Local Docs Preview",
  homeIntro:
    "Local documentation preview for install, device identity, and release flow notes in the MundusX repo.",
});
writeDocsVariant({
  outputBaseDir: path.join(outputDir, "public", "docs"),
  eyebrow: "Public Endpoint Mirror",
  homeIntro:
    "GitHub Pages mirror of the public docs shape so install, device identity, and release pages can be reviewed before the final domain is wired up.",
});
writeMarketingHomepage();
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

function writeMarketingHomepage() {
  const assetDir = path.join(outputDir, "assets");
  fs.mkdirSync(assetDir, { recursive: true });
  fs.copyFileSync(path.join(repoRoot, "media/homepage-dark-bg.png"), path.join(assetDir, "homepage-dark-bg.png"));
  fs.copyFileSync(path.join(repoRoot, "media/mundusx-logo-official.svg"), path.join(assetDir, "mundusx-logo-official.svg"));
  fs.writeFileSync(path.join(outputDir, "index.html"), renderMarketingHomePage());
}

function renderMarketingHomePage() {
  return `<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <title>MundusX</title>
    <style>
      :root {
        color-scheme: dark;
        --page: #020910;
        --surface: rgba(5, 23, 37, 0.76);
        --surface-strong: rgba(3, 15, 27, 0.94);
        --surface-soft: rgba(6, 29, 47, 0.72);
        --border: rgba(91, 211, 255, 0.24);
        --border-strong: rgba(91, 211, 255, 0.48);
        --text: #f4f8ff;
        --muted: #b7c8d7;
        --soft: #8da5b6;
        --accent: #31cfff;
        --accent-2: #74e6ff;
        --accent-3: #168bff;
        --glow: rgba(49, 207, 255, 0.34);
        --shadow: 0 28px 80px rgba(0, 0, 0, 0.36);
        --hero-overlay: linear-gradient(90deg, rgba(2,9,16,0.92), rgba(2,12,21,0.36) 46%, rgba(2,9,16,0.82));
        --waves: radial-gradient(circle at 78% 9%, rgba(38, 158, 237, 0.34), transparent 25rem),
          radial-gradient(circle at 13% 78%, rgba(31, 206, 255, 0.14), transparent 25rem),
          linear-gradient(180deg, #03121d 0%, #020811 50%, #030e18 100%);
        --nav-bg: rgba(2, 12, 22, 0.82);
        --icon-bg: rgba(24, 96, 134, 0.24);
        --mountain: linear-gradient(135deg, transparent 0 38%, rgba(104, 178, 238, 0.22) 39% 62%, transparent 63%),
          linear-gradient(45deg, transparent 0 40%, rgba(1, 92, 178, 0.18) 41% 58%, transparent 59%);
      }
      * { box-sizing: border-box; }
      html {
        background: var(--page);
        scroll-behavior: smooth;
      }
      body {
        margin: 0;
        min-height: 100vh;
        font-family: Inter, ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
        letter-spacing: 0;
        color: var(--text);
        background: var(--waves);
        overflow-x: hidden;
      }
      body::before {
        content: "";
        position: fixed;
        inset: 0;
        pointer-events: none;
        background:
          radial-gradient(circle at 14% 36%, color-mix(in srgb, var(--accent) 20%, transparent), transparent 1px),
          linear-gradient(90deg, color-mix(in srgb, var(--accent) 5%, transparent) 1px, transparent 1px),
          linear-gradient(180deg, color-mix(in srgb, var(--accent) 4%, transparent) 1px, transparent 1px);
        background-size: 18px 18px, 44px 44px, 44px 44px;
        mask-image: linear-gradient(180deg, transparent, #000 16%, #000 84%, transparent);
        opacity: 0.65;
      }
      a {
        color: inherit;
        text-decoration: none;
      }
      svg {
        display: block;
        stroke: currentColor;
        stroke-linecap: round;
        stroke-linejoin: round;
      }
      .topbar {
        position: sticky;
        top: 0;
        z-index: 20;
        border-bottom: 1px solid rgba(91, 211, 255, 0.1);
        background: var(--nav-bg);
        backdrop-filter: blur(22px);
      }
      .nav {
        width: min(1720px, calc(100% - 76px));
        height: 64px;
        margin: 0 auto;
        display: flex;
        align-items: center;
        justify-content: space-between;
        gap: 28px;
      }
      .brand {
        display: flex;
        align-items: center;
        gap: 15px;
        font-size: 25px;
        font-weight: 800;
        letter-spacing: -0.01em;
      }
      .mark {
        width: 47px;
        height: 30px;
        display: block;
        object-fit: contain;
        filter: drop-shadow(0 0 15px var(--glow));
      }
      .links {
        display: flex;
        align-items: center;
        gap: clamp(22px, 3vw, 48px);
        color: #edf6ff;
        font-size: 14px;
        font-weight: 600;
      }
      .actions {
        display: flex;
        gap: 12px;
        align-items: center;
      }
      .btn {
        min-height: 46px;
        display: inline-flex;
        align-items: center;
        justify-content: center;
        gap: 12px;
        border-radius: 999px;
        border: 1px solid var(--border-strong);
        padding: 0 28px;
        font-size: 14px;
        font-weight: 800;
        white-space: nowrap;
        transition: transform 160ms ease, box-shadow 160ms ease, border-color 160ms ease;
      }
      .btn:hover {
        transform: translateY(-1px);
      }
      .btn.primary {
        color: white;
        background: linear-gradient(135deg, #35c8ff, #006eff);
        border-color: transparent;
        box-shadow: 0 18px 38px var(--glow), inset 0 1px rgba(255, 255, 255, 0.48);
      }
      .btn.ghost {
        background: var(--surface);
        box-shadow: inset 0 1px rgba(255, 255, 255, 0.35);
      }
      .theme-toggle {
        display: none;
        width: 46px;
        height: 46px;
        border: 1px solid var(--border);
        border-radius: 999px;
        color: var(--accent);
        background: var(--surface);
        display: grid;
        place-items: center;
        cursor: pointer;
        box-shadow: 0 10px 28px rgba(28, 114, 199, 0.09);
      }
      .hero {
        position: relative;
        min-height: 690px;
        overflow: hidden;
      }
      .hero::before {
        content: "";
        position: absolute;
        inset: 0;
        background:
          var(--hero-overlay),
          radial-gradient(ellipse at 74% 12%, rgba(37, 181, 255, 0.18), transparent 27rem),
          url("./assets/homepage-dark-bg.png") center 50% / cover no-repeat;
        opacity: 1;
      }
      .hero::after {
        content: "";
        position: absolute;
        inset: 0;
        background:
          radial-gradient(ellipse at 51% 48%, transparent 0 26%, rgba(2, 9, 16, 0.2) 45%, rgba(2, 9, 16, 0.78) 100%),
          linear-gradient(180deg, transparent 0 74%, rgba(2, 9, 16, 0.9) 100%);
        opacity: 0.76;
      }
      .hero-inner {
        position: relative;
        z-index: 1;
        width: min(1700px, calc(100% - 92px));
        margin: 0 auto;
        padding: 58px 0 72px;
        display: grid;
        grid-template-columns: minmax(560px, 0.78fr) minmax(900px, 1.22fr);
        gap: 108px;
        align-items: center;
      }
      .pill {
        display: inline-flex;
        align-items: center;
        border: 1px solid var(--border-strong);
        border-radius: 999px;
        padding: 9px 15px;
        color: var(--accent);
        background: rgba(3, 26, 43, 0.7);
        font-size: 13px;
        font-weight: 800;
        letter-spacing: 0.14em;
        text-transform: uppercase;
      }
      h1 {
        max-width: 820px;
        margin: 28px 0 22px;
        font-size: clamp(58px, 4.2vw, 76px);
        line-height: 1;
        letter-spacing: -0.025em;
      }
      h1 span {
        color: #66d8ff;
      }
      .lead {
        max-width: 600px;
        margin: 0 0 32px;
        color: #e4eef8;
        font-size: clamp(19px, 1.2vw, 23px);
        line-height: 1.28;
      }
      .hero-buttons {
        display: flex;
        flex-wrap: wrap;
        gap: 18px;
        margin-bottom: 22px;
      }
      .hero-buttons .btn {
        min-width: 286px;
      }
      .notes {
        display: flex;
        flex-wrap: wrap;
        gap: 18px;
        color: var(--soft);
        font-size: 14px;
        font-weight: 600;
      }
      .notes span {
        display: inline-flex;
        align-items: center;
        gap: 8px;
      }
      .notes span::before {
        display: none;
      }
      .notes span + span::before {
        content: "";
        display: inline-block;
        width: 4px;
        height: 4px;
        margin: 0 11px 3px 0;
        border-radius: 50%;
        background: var(--accent);
        box-shadow: 0 0 14px var(--accent);
      }
      .product-shell {
        display: grid;
        grid-template-columns: 288px 1fr;
        min-height: 496px;
        border: 1px solid var(--border-strong);
        border-radius: 20px;
        background: rgba(3, 16, 27, 0.92);
        box-shadow: 0 0 44px rgba(31, 201, 255, 0.36), inset 0 1px rgba(255, 255, 255, 0.1);
        overflow: hidden;
      }
      .sidebar {
        padding: 20px 22px;
        border-right: 1px solid var(--border);
        background: rgba(2, 12, 22, 0.72);
      }
      .mini-brand {
        display: flex;
        align-items: center;
        gap: 10px;
        font-size: 14px;
        font-weight: 800;
      }
      .mini-mark {
        width: 29px;
        height: 22px;
        display: block;
        object-fit: contain;
      }
      .mini-kicker {
        display: block;
        margin-top: 1px;
        color: var(--soft);
        font-size: 7px;
        letter-spacing: 0.18em;
        text-transform: uppercase;
      }
      .new-chat {
        width: 100%;
        margin: 20px 0 18px;
        min-height: 40px;
        justify-content: flex-start;
        border-radius: 9px;
        padding: 0 16px;
      }
      .section-label {
        margin: 18px 0 8px;
        color: var(--soft);
        font-size: 9px;
        font-weight: 800;
        letter-spacing: 0.16em;
        text-transform: uppercase;
      }
      .side-item,
      .chat-row {
        display: flex;
        align-items: center;
        gap: 10px;
        min-height: 26px;
        color: #d7e5f0;
        font-size: 12px;
      }
      .side-item svg,
      .chat-row svg {
        width: 15px;
        height: 15px;
        color: var(--accent);
      }
      .recent {
        margin-top: 12px;
      }
      .chat-row {
        justify-content: space-between;
        color: #c7d8e5;
      }
      .chat-row b {
        min-width: 0;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
        font-weight: 500;
      }
      .chat-row time {
        color: var(--soft);
        font-size: 10px;
      }
      .app-main {
        position: relative;
        display: flex;
        flex-direction: column;
        justify-content: center;
        align-items: center;
        padding: 82px 42px 58px;
      }
      .app-main::after {
        content: "DRIVING INTELLIGENCE FOR A BRIGHTER TOMORROW";
        position: absolute;
        top: 92px;
        right: 38px;
        width: 104px;
        color: color-mix(in srgb, var(--soft) 72%, transparent);
        font-size: 9px;
        line-height: 1.7;
        letter-spacing: 0.36em;
      }
      .toggle {
        position: absolute;
        top: 14px;
        right: 16px;
        display: flex;
        gap: 7px;
        padding: 5px;
        border-radius: 999px;
        border: 1px solid var(--border);
        background: var(--surface-soft);
      }
      .toggle span {
        width: 27px;
        height: 27px;
        display: grid;
        place-items: center;
        border-radius: 50%;
        color: var(--accent);
      }
      .toggle span:first-child {
        color: white;
        background: linear-gradient(135deg, var(--accent-2), var(--accent-3));
        box-shadow: 0 8px 18px var(--glow);
      }
      .assistant-title {
        margin-top: 4px;
        text-align: center;
      }
      .assistant-title h2 {
        margin: 0;
        font-size: 25px;
        line-height: 1.1;
        letter-spacing: -0.02em;
      }
      .assistant-title span {
        color: var(--accent);
      }
      .assistant-title p {
        max-width: 500px;
        margin: 12px auto 0;
        color: #cfdeea;
        font-size: 13px;
        line-height: 1.48;
      }
      .tool-grid {
        width: 100%;
        display: grid;
        grid-template-columns: repeat(4, 1fr);
        gap: 14px;
        margin: 36px 0 52px;
      }
      .tool-card {
        min-height: 148px;
        padding: 28px 16px 18px;
        border: 1px solid rgba(91, 211, 255, 0.24);
        border-radius: 8px;
        background: linear-gradient(180deg, rgba(7, 38, 59, 0.9), rgba(4, 21, 36, 0.82));
        text-align: center;
        box-shadow: inset 0 1px rgba(255, 255, 255, 0.26);
      }
      .tool-icon {
        width: 34px;
        height: 34px;
        display: grid;
        place-items: center;
        margin: 0 auto 20px;
        color: var(--accent);
      }
      .tool-icon svg { width: 36px; height: 36px; stroke-width: 2.4; }
      .tool-card b {
        display: block;
        font-size: 13px;
      }
      .tool-card p {
        margin: 8px auto 0;
        color: #aebfce;
        font-size: 11px;
        line-height: 1.35;
      }
      .prompt {
        width: 100%;
        min-height: 58px;
        display: grid;
        grid-template-columns: 28px 1fr repeat(4, 30px) 46px;
        gap: 14px;
        align-items: center;
        border: 1px solid rgba(91, 211, 255, 0.26);
        border-radius: 999px;
        background: rgba(5, 30, 50, 0.88);
        padding: 6px 8px 6px 18px;
        color: var(--soft);
        font-size: 12px;
        box-shadow: 0 18px 48px rgba(18, 83, 156, 0.08);
      }
      .prompt svg {
        width: 18px;
        height: 18px;
      }
      .send {
        width: 46px;
        height: 46px;
        display: grid;
        place-items: center;
        border-radius: 50%;
        color: white;
        background: linear-gradient(135deg, var(--accent-2), var(--accent-3));
        box-shadow: 0 10px 25px var(--glow);
      }
      .feature-rail {
        position: relative;
        z-index: 2;
        width: min(1920px, calc(100% - 92px));
        margin: 40px auto 0;
        display: grid;
        grid-template-columns: repeat(4, 1fr);
        gap: 24px;
      }
      .rail-card {
        min-height: 96px;
        display: grid;
        grid-template-columns: 66px 1fr 18px;
        gap: 22px;
        align-items: center;
        padding: 18px 32px;
        border: 1px solid rgba(91, 211, 255, 0.3);
        border-radius: 8px;
        background: rgba(7, 32, 52, 0.94);
        box-shadow: inset 0 1px rgba(255, 255, 255, 0.08), 0 0 26px rgba(36, 174, 229, 0.16);
      }
      .rail-icon {
        width: 50px;
        height: 50px;
        display: grid;
        place-items: center;
        border-radius: 8px;
        color: var(--accent);
        background: rgba(35, 206, 255, 0.09);
        box-shadow: 0 0 20px rgba(50, 212, 255, 0.46);
      }
      .rail-icon svg {
        width: 32px;
        height: 32px;
        stroke-width: 2.2;
      }
      .rail-card h3 {
        margin: 0 0 4px;
        font-size: 18px;
      }
      .rail-card p {
        margin: 0;
        color: #bdd1df;
        font-size: 13px;
        line-height: 1.35;
      }
      .network {
        position: relative;
        height: clamp(280px, 14.2vw, 340px);
        margin-top: 66px;
        overflow: hidden;
        background:
          radial-gradient(circle at 51.5% 58%, rgba(8, 102, 154, 0.34), transparent 19%),
          radial-gradient(circle at 51.5% 54%, rgba(32, 199, 247, 0.12), transparent 28%),
          linear-gradient(180deg, rgba(2, 13, 22, 0.98), rgba(3, 18, 30, 0.96));
      }
      .network::before {
        content: "";
        position: absolute;
        inset: 0;
        background:
          linear-gradient(90deg, transparent 0%, rgba(32, 151, 199, 0.07) 25%, rgba(32, 151, 199, 0.08) 52%, rgba(32, 151, 199, 0.05) 78%, transparent 100%),
          linear-gradient(90deg, rgba(32, 151, 199, 0.065) 1px, transparent 1px),
          linear-gradient(180deg, rgba(32, 151, 199, 0.055) 1px, transparent 1px),
          url("./assets/homepage-dark-bg.png") center 61% / cover no-repeat;
        background-size: auto, 66px 66px, 66px 66px, cover;
        mask-image: linear-gradient(90deg, transparent 0%, #000 16%, #000 84%, transparent 100%);
        opacity: 0.62;
      }
      .network::after {
        content: "";
        position: absolute;
        inset: 0;
        pointer-events: none;
        background:
          radial-gradient(circle at 28% 36%, rgba(50, 213, 255, 0.4) 0 1px, transparent 2px),
          radial-gradient(circle at 39% 52%, rgba(50, 213, 255, 0.34) 0 1px, transparent 2px),
          radial-gradient(circle at 58% 31%, rgba(50, 213, 255, 0.46) 0 1px, transparent 2px),
          radial-gradient(circle at 71% 56%, rgba(50, 213, 255, 0.36) 0 1px, transparent 2px);
        opacity: 0.65;
      }
      .network-inner {
        position: absolute;
        inset: 0;
        z-index: 1;
        width: 100%;
        max-width: 2048px;
        height: 100%;
        margin: 0 auto;
        left: 50%;
        transform: translateX(-50%);
      }
      .device {
        position: absolute;
        left: 10.5%;
        top: 48%;
        width: 230px;
        transform: translate(-50%, -50%);
        z-index: 2;
        text-align: center;
      }
      .laptop {
        position: relative;
        width: 226px;
        height: 134px;
        margin: 0 auto 10px;
        color: var(--accent);
      }
      .laptop svg {
        display: none;
      }
      .laptop::before {
        content: "";
        position: absolute;
        left: 13px;
        top: 7px;
        width: 198px;
        height: 105px;
        border: 2px solid rgba(40, 200, 245, 0.72);
        border-radius: 6px 6px 2px 2px;
        background:
          linear-gradient(142deg, rgba(64, 177, 239, 0.12), transparent 42%),
          linear-gradient(150deg, transparent 0 45%, rgba(10, 58, 86, 0.54) 46% 63%, transparent 64%),
          rgba(2, 10, 18, 0.86);
        box-shadow: 0 0 22px rgba(49, 207, 255, 0.28), inset 0 0 24px rgba(49, 207, 255, 0.06);
      }
      .laptop::after {
        content: "";
        position: absolute;
        left: 0;
        bottom: 9px;
        width: 226px;
        height: 17px;
        border-radius: 50% 50% 7px 7px;
        background: linear-gradient(90deg, transparent, rgba(112, 218, 255, 0.68) 9% 90%, transparent);
        box-shadow: 0 8px 15px rgba(49, 207, 255, 0.18);
      }
      .device h2 {
        margin: -2px 0 5px;
        color: #f3f8fc;
        font-size: 20px;
        line-height: 1.1;
      }
      .device p {
        margin: 0;
        color: #9db2c5;
        font-size: 16px;
        font-weight: 650;
      }
      .network-lines {
        position: absolute;
        inset: 0;
        z-index: 1;
        width: 100%;
        height: 100%;
        color: var(--accent);
        overflow: visible;
        pointer-events: none;
      }
      .network-lines path {
        fill: none;
        stroke: currentColor;
        stroke-width: 1.8;
        opacity: 0.5;
        filter: url(#pathGlow);
      }
      .network-lines .axis {
        opacity: 0.36;
      }
      .network-lines circle {
        fill: #8ceaff;
        filter: url(#nodeGlow);
      }
      .network-core {
        position: absolute;
        left: 51.5%;
        top: 62%;
        width: 360px;
        height: 360px;
        transform: translate(-50%, -50%);
        display: grid;
        place-items: center;
        text-align: center;
        border-radius: 50%;
        background: radial-gradient(circle, rgba(8, 127, 181, 0.18), rgba(8, 127, 181, 0.05) 49%, transparent 72%);
        box-shadow: 0 0 42px rgba(37, 211, 255, 0.28);
        overflow: hidden;
      }
      .globe-mesh {
        position: absolute;
        inset: 0;
        width: 100%;
        height: 100%;
        color: #25d3ff;
        opacity: 0.94;
      }
      .network-core .mark {
        margin: 0 auto 11px;
        width: 62px;
        height: 42px;
      }
      .network h2,
      .network p {
        margin: 0;
      }
      .network h2 {
        color: #f3f8fc;
        font-size: 27px;
        line-height: 1.1;
      }
      .network h2 span { color: var(--accent); }
      .network p {
        margin-top: 22px;
        color: #9db2c5;
        font-size: 18px;
        line-height: 1.38;
        font-weight: 650;
      }
      .network-core-content {
        position: relative;
        z-index: 2;
        width: 270px;
        padding: 20px 12px 22px;
        background: radial-gradient(ellipse at center, rgba(3, 18, 30, 0.72), rgba(3, 18, 30, 0.32) 56%, transparent 78%);
      }
      .server-and-panel {
        position: absolute;
        left: 86%;
        top: 50%;
        width: 28%;
        transform: translate(-50%, -50%);
        z-index: 2;
      }
      .server-stack {
        position: absolute;
        left: 30%;
        top: 50%;
        transform: translate(-50%, -50%);
        display: grid;
        gap: 9px;
      }
      .server-stack span {
        width: 80px;
        height: 28px;
        border: 1px solid rgba(91, 211, 255, 0.34);
        border-radius: 6px;
        background: linear-gradient(180deg, rgba(33, 126, 171, 0.26), rgba(5, 24, 39, 0.88));
        box-shadow: inset 0 1px rgba(255, 255, 255, 0.08), 0 0 14px rgba(49, 207, 255, 0.16);
        position: relative;
      }
      .server-stack span::before,
      .server-stack span::after {
        content: "";
        position: absolute;
        top: 10px;
        width: 4px;
        height: 4px;
        border-radius: 50%;
        background: var(--accent);
        box-shadow: 0 0 8px var(--accent);
      }
      .server-stack span::before {
        left: 10px;
      }
      .server-stack span::after {
        right: 10px;
        opacity: 0.45;
      }
      .node-list {
        width: 310px;
        height: 225px;
        margin-left: auto;
        border: 1px solid rgba(18, 168, 218, 0.45);
        border-radius: 15px;
        background: rgba(4, 25, 39, 0.9);
        text-align: left;
        box-shadow: 0 0 25px rgba(0, 174, 239, 0.08), inset 0 1px rgba(255, 255, 255, 0.06);
      }
      .node-list div {
        display: flex;
        align-items: center;
        gap: 28px;
        min-height: 74px;
        padding: 0 32px;
        border-bottom: 1px solid rgba(73, 148, 180, 0.22);
        color: #f3f8fc;
        font-size: 19px;
        font-weight: 650;
      }
      .node-list div:last-child { border-bottom: 0; }
      .node-list svg {
        width: 40px;
        height: 40px;
        color: #29c8fa;
        stroke-width: 1.9;
        filter: drop-shadow(0 0 10px rgba(41, 200, 250, 0.42));
      }
      .values {
        position: relative;
        padding: 84px 0 0;
        background:
          linear-gradient(180deg, rgba(2, 9, 16, 0.98), rgba(3, 13, 23, 0.9)),
          url("./assets/homepage-dark-bg.png") center bottom / cover no-repeat;
        background-blend-mode: normal, luminosity;
      }
      .values-inner {
        width: min(1680px, calc(100% - 92px));
        margin: 0 auto;
        text-align: center;
      }
      .values h2 {
        margin: 0;
        font-size: clamp(38px, 3.05vw, 56px);
        line-height: 1.05;
        letter-spacing: -0.025em;
      }
      .values > .values-inner > p {
        margin: 14px auto 36px;
        max-width: 760px;
        color: #b7c7d6;
        font-size: 20px;
        line-height: 1.35;
      }
      .value-grid {
        display: grid;
        grid-template-columns: repeat(3, 1fr);
        gap: 0;
        text-align: left;
      }
      .value {
        display: grid;
        grid-template-columns: 118px 1fr;
        gap: 32px;
        align-items: center;
        min-height: 124px;
        padding: 0 58px;
      }
      .value:not(:last-child) {
        border-right: 1px solid rgba(91, 211, 255, 0.18);
      }
      .value-icon {
        width: 104px;
        height: 104px;
        display: grid;
        place-items: center;
        border-radius: 50%;
        color: var(--accent);
        border: 1px solid rgba(91, 211, 255, 0.13);
        background: radial-gradient(circle, rgba(36, 161, 221, 0.2), rgba(8, 47, 73, 0.2) 58%, rgba(4, 22, 37, 0.42));
        box-shadow: inset 0 0 38px rgba(49, 207, 255, 0.08), 0 0 32px rgba(49, 207, 255, 0.09);
      }
      .value-icon svg {
        width: 46px;
        height: 46px;
        stroke-width: 1.9;
      }
      .value h3 {
        margin: 0 0 12px;
        color: #f3f8ff;
        font-size: 23px;
        line-height: 1.12;
        letter-spacing: -0.01em;
      }
      .value p {
        margin: 0;
        color: #c6d4e1;
        font-size: 18px;
        line-height: 1.35;
      }
      .cta {
        position: relative;
        z-index: 1;
        width: min(980px, 100%);
        min-height: 132px;
        margin: 62px auto 0;
        display: grid;
        grid-template-columns: 1fr auto;
        gap: 38px;
        align-items: center;
        text-align: left;
        padding: 28px 38px;
        border: 1px solid var(--border);
        border-radius: 8px;
        background: var(--surface-strong);
        box-shadow: var(--shadow);
      }
      .cta h2 {
        margin: 0 0 8px;
        font-size: 24px;
      }
      .cta p,
      .cta small {
        margin: 0;
        color: var(--muted);
      }
      .cta small {
        display: block;
        margin-top: 10px;
        text-align: center;
        font-size: 11px;
      }
      .mountains {
        height: 135px;
        margin-top: -44px;
        background:
          var(--mountain),
          linear-gradient(180deg, transparent, color-mix(in srgb, var(--accent) 9%, transparent));
        opacity: 0.72;
        clip-path: polygon(0 55%, 7% 44%, 12% 49%, 20% 28%, 27% 46%, 37% 34%, 44% 48%, 52% 30%, 61% 48%, 70% 24%, 78% 46%, 88% 30%, 100% 54%, 100% 100%, 0 100%);
      }
      .site-footer {
        border-top: 1px solid var(--border);
        background: var(--surface-strong);
      }
      .footer-inner {
        width: min(1320px, calc(100% - 72px));
        min-height: 88px;
        margin: 0 auto;
        display: flex;
        align-items: center;
        justify-content: space-between;
        gap: 28px;
      }
      .footer-links,
      .social-links {
        display: flex;
        align-items: center;
        gap: 26px;
        color: var(--muted);
        font-size: 13px;
        font-weight: 600;
      }
      .social-links {
        gap: 16px;
        color: var(--accent);
      }
      .social-links svg {
        width: 20px;
        height: 20px;
      }
      .tagline {
        color: var(--soft);
        font-size: 10px;
        letter-spacing: 0.24em;
        text-transform: uppercase;
      }
      @media (max-width: 1120px) {
        .hero-inner {
          grid-template-columns: 1fr;
          gap: 42px;
        }
        .product-shell {
          max-width: 780px;
          width: 100%;
          margin: 0 auto;
        }
        .feature-rail {
          grid-template-columns: repeat(2, 1fr);
          margin-top: 34px;
        }
        .network {
          height: clamp(280px, 25vw, 340px);
        }
        .value-grid {
          grid-template-columns: repeat(2, 1fr);
        }
        .value:not(:last-child) {
          border-right: 0;
          padding-right: 0;
        }
      }
      @media (min-width: 1121px) and (max-width: 1540px) {
        .nav,
        .hero-inner,
        .feature-rail {
          width: min(1348px, calc(100% - 92px));
        }
        .hero-inner {
          grid-template-columns: minmax(500px, 0.78fr) minmax(720px, 1.22fr);
          gap: 64px;
        }
        h1 {
          font-size: clamp(56px, 4.9vw, 76px);
        }
        .lead {
          max-width: 560px;
        }
        .product-shell {
          grid-template-columns: 238px 1fr;
          min-height: 454px;
        }
        .app-main {
          padding: 70px 30px 48px;
        }
        .app-main::after {
          display: none;
        }
        .tool-grid {
          gap: 12px;
          margin: 30px 0 40px;
        }
        .tool-card {
          min-height: 132px;
          padding: 22px 12px 16px;
        }
        .feature-rail {
          gap: 18px;
        }
        .rail-card {
          padding: 18px 22px;
        }
      }
      @media (max-width: 760px) {
        .nav {
          width: min(100% - 28px, 1420px);
          height: 64px;
        }
        .links {
          display: none;
        }
        .actions .ghost {
          display: none;
        }
        .hero-inner,
        .feature-rail,
        .network-inner,
        .values-inner,
        .footer-inner {
          width: min(100% - 28px, 1240px);
        }
        .hero {
          min-height: auto;
        }
        .hero-inner {
          padding: 54px 0 76px;
          gap: 44px;
        }
        .product-shell {
          grid-template-columns: 1fr;
        }
        .sidebar {
          display: none;
        }
        .app-main::after {
          display: none;
        }
        .tool-grid {
          grid-template-columns: repeat(2, 1fr);
        }
        .prompt {
          grid-template-columns: 22px 1fr 40px;
        }
        .prompt span:nth-last-child(n+2):nth-last-child(-n+5) {
          display: none;
        }
        .feature-rail,
        .network-inner,
        .value-grid,
        .cta {
          grid-template-columns: 1fr;
        }
        .feature-rail {
          margin-top: 30px;
        }
        .network {
          height: 620px;
          margin-top: 48px;
        }
        .network-inner {
          width: 100%;
        }
        .network-lines {
          display: none;
        }
        .device {
          left: 50%;
          top: 16%;
          transform: translate(-50%, -50%) scale(0.72);
        }
        .network-core {
          left: 50%;
          top: 48%;
          width: 290px;
          height: 290px;
        }
        .network-core .mark {
          width: 50px;
          height: 34px;
        }
        .network h2 {
          font-size: 22px;
        }
        .network p {
          font-size: 15px;
          margin-top: 14px;
        }
        .server-and-panel {
          left: 50%;
          top: 84%;
          width: 310px;
          transform: translate(-50%, -50%) scale(0.86);
        }
        .server-stack {
          display: none;
        }
        .values {
          padding-top: 64px;
        }
        .value:not(:last-child) {
          border-right: 0;
          padding-right: 0;
        }
        .footer-inner,
        .footer-links {
          flex-direction: column;
          align-items: flex-start;
          padding: 24px 0;
        }
        .footer-links {
          gap: 14px;
          padding: 0;
        }
      }
      @media (max-width: 480px) {
        .brand {
          font-size: 21px;
        }
        .mark {
          width: 36px;
          height: 27px;
        }
        .theme-toggle {
          display: none;
        }
        .btn {
          padding: 0 17px;
        }
        .hero-buttons .btn {
          width: 100%;
        }
        .notes {
          display: grid;
          gap: 8px;
        }
        .notes span + span::before {
          display: none;
        }
        .tool-card {
          min-height: 126px;
        }
        .rail-card {
          padding: 18px;
        }
        .cta {
          padding: 20px;
        }
      }
    </style>
  </head>
  <body>
    <header class="topbar">
      <nav class="nav" aria-label="Primary">
        <a class="brand" href="./" aria-label="MundusX home">${brandMark()}<span>MundusX</span></a>
        <div class="links" aria-label="Sections">
          <a href="./public/docs/">Product</a>
          <a href="./public/release/">Network</a>
          <a href="./docs/">Developers</a>
          <a href="./public/install/">Pricing</a>
          <a href="./docs/device-identity/">About</a>
        </div>
        <div class="actions">
          <a class="btn ghost" href="./public/docs/">Sign in</a>
          <a class="btn primary" href="./public/install/">Get started <span aria-hidden="true">-&gt;</span></a>
        </div>
      </nav>
    </header>
    <main>
      <section class="hero" aria-labelledby="home-title">
        <div class="hero-inner">
          <div>
            <span class="pill">Decentralized AI Network</span>
            <h1 id="home-title">AI that works across <span>your world.</span></h1>
            <p class="lead">Chat, build, research and run AI workloads across a secure decentralized compute network - with your projects and tools connected.</p>
            <div class="hero-buttons">
              <a class="btn primary" href="./public/install/">Start with MundusX <span aria-hidden="true">-&gt;</span></a>
              <a class="btn ghost" href="./public/release/">Explore the network</a>
            </div>
            <div class="notes"><span>Start free</span><span>Sign in with Google</span><span>Connect your own computer</span></div>
          </div>
          <div class="product-shell" aria-label="Atlas workspace preview">
            <aside class="sidebar">
              <div class="mini-brand">${brandMark("mini-mark")}<span>MundusX <small class="mini-kicker">A better tomorrow</small></span></div>
              <div class="btn primary new-chat">+ New Chat</div>
              <div class="section-label">Workspace</div>
              <div class="side-item">${siteIcon("folder")}Projects</div>
              <div class="side-item">${siteIcon("folder")}Product Ideas</div>
              <div class="side-item">${siteIcon("folder")}AI Research</div>
              <div class="side-item">${siteIcon("folder")}Customer Support</div>
              <div class="side-item">${siteIcon("folder")}Personal</div>
              <div class="section-label">Recent Chats</div>
              <div class="recent">
                <div class="chat-row">${siteIcon("folder")}<b>How to optimize LLM...</b><time>12:20 AM</time></div>
                <div class="chat-row">${siteIcon("folder")}<b>Generate project plan</b><time>10:14 AM</time></div>
                <div class="chat-row">${siteIcon("folder")}<b>Explain RISC-V</b><time>Sep 9</time></div>
                <div class="chat-row">${siteIcon("folder")}<b>Build a pricing model</b><time>Sep 9</time></div>
                <div class="chat-row">${siteIcon("folder")}<b>Summarize this doc</b><time>Sep 8</time></div>
              </div>
            </aside>
            <div class="app-main">
              <div class="toggle"><span>${siteIcon("sun")}</span><span>${siteIcon("moon")}</span></div>
              <div class="assistant-title">
                <h2>Hello, my name is <span>Atlas.</span></h2>
                <p>I can help you chat, write code, analyze data, build solutions and research anything - across the MundusX network.</p>
              </div>
              <div class="tool-grid">
                <div class="tool-card"><span class="tool-icon">${siteIcon("code")}</span><b>Code</b><p>Write, debug and explain code</p></div>
                <div class="tool-card"><span class="tool-icon">${siteIcon("bar-chart")}</span><b>Analyze</b><p>Turn data into insights</p></div>
                <div class="tool-card"><span class="tool-icon">${siteIcon("layers")}</span><b>Build</b><p>Create and plan projects</p></div>
                <div class="tool-card"><span class="tool-icon">${siteIcon("search")}</span><b>Research</b><p>Explore ideas and get deeper answers</p></div>
              </div>
              <div class="prompt"><span>${siteIcon("paperclip")}</span><span>Ask Atlas anything...</span><span>${siteIcon("globe")}</span><span>${siteIcon("sliders")}</span><span>${siteIcon("mic")}</span><span>${siteIcon("spark")}</span><span class="send">${siteIcon("arrow-up")}</span></div>
            </div>
          </div>
        </div>
      </section>
      <section class="feature-rail" aria-label="MundusX features">
        <a class="rail-card" href="./public/docs/"><span class="rail-icon">${siteIcon("message-square")}</span><div><h3>Chat with Atlas</h3><p>Your AI companion for work, ideas and everyday tasks.</p></div><span aria-hidden="true">&gt;</span></a>
        <a class="rail-card" href="./docs/"><span class="rail-icon">${siteIcon("folder")}</span><div><h3>Projects</h3><p>Keep chats, files and work together in one place.</p></div><span aria-hidden="true">&gt;</span></a>
        <a class="rail-card" href="./docs/install/"><span class="rail-icon">${siteIcon("terminal")}</span><div><h3>Harness</h3><p>Run code on your own computer</p></div><span aria-hidden="true">&gt;</span></a>
        <a class="rail-card" href="./public/release/"><span class="rail-icon">${siteIcon("network")}</span><div><h3>Decentralized Compute</h3><p>Contribute or use distributed compute.</p></div><span aria-hidden="true">&gt;</span></a>
      </section>
      <section class="network" aria-label="Decentralized compute network">
        <div class="network-inner">
          <svg class="network-lines" viewBox="0 0 2048 290" preserveAspectRatio="none" aria-hidden="true">
            <defs>
              <filter id="pathGlow" x="-30%" y="-120%" width="160%" height="340%">
                <feGaussianBlur stdDeviation="3.8" result="blur" />
                <feMerge><feMergeNode in="blur" /><feMergeNode in="SourceGraphic" /></feMerge>
              </filter>
              <filter id="nodeGlow" x="-220%" y="-220%" width="540%" height="540%">
                <feGaussianBlur stdDeviation="5" result="blur" />
                <feMerge><feMergeNode in="blur" /><feMergeNode in="SourceGraphic" /></feMerge>
              </filter>
            </defs>
            <path class="axis" d="M260 146 C500 146 666 146 841 146 M1207 146 C1352 146 1474 146 1694 146" />
            <path d="M258 122 C372 78 476 75 563 102 C643 127 688 150 841 146" />
            <path d="M258 156 C371 188 477 184 566 162 C653 141 714 151 841 146" />
            <path d="M1207 146 C1328 148 1398 82 1517 79 C1588 77 1628 94 1694 96" />
            <path d="M1207 146 C1331 148 1394 171 1494 168 C1579 165 1630 156 1694 158" />
            <circle cx="258" cy="122" r="5" /><circle cx="490" cy="86" r="7" /><circle cx="581" cy="126" r="6" /><circle cx="660" cy="145" r="6" /><circle cx="468" cy="176" r="7" />
            <circle cx="1212" cy="146" r="6" /><circle cx="1394" cy="146" r="6" /><circle cx="1481" cy="111" r="6" /><circle cx="1535" cy="86" r="7" /><circle cx="1518" cy="168" r="6" />
          </svg>
          <div class="device"><div class="laptop">${siteIcon("laptop")}</div><h2>Your Device</h2><p>Work from anywhere</p></div>
          <div class="network-core">
            <svg class="globe-mesh" viewBox="0 0 360 360" preserveAspectRatio="xMidYMid meet" aria-hidden="true">
              <defs>
                <filter id="globeGlow" x="-40%" y="-40%" width="180%" height="180%">
                  <feGaussianBlur stdDeviation="3.4" result="blur" />
                  <feMerge><feMergeNode in="blur" /><feMergeNode in="SourceGraphic" /></feMerge>
                </filter>
              </defs>
              <circle cx="180" cy="180" r="172" fill="rgba(2, 13, 22, 0.08)" stroke="rgba(37, 211, 255, 0.62)" stroke-width="2.2" filter="url(#globeGlow)" />
              <ellipse cx="180" cy="180" rx="116" ry="172" fill="none" stroke="rgba(37, 211, 255, 0.25)" stroke-width="1.2" />
              <ellipse cx="180" cy="180" rx="58" ry="172" fill="none" stroke="rgba(37, 211, 255, 0.18)" stroke-width="1" />
              <ellipse cx="180" cy="180" rx="172" ry="62" fill="none" stroke="rgba(37, 211, 255, 0.18)" stroke-width="1" />
              <ellipse cx="180" cy="180" rx="172" ry="118" fill="none" stroke="rgba(37, 211, 255, 0.15)" stroke-width="1" />
              <path d="M52 74 115 32 183 21 251 47 310 101 330 174 305 244 238 317 158 336 80 303 35 230 28 145Z" fill="none" stroke="rgba(37,211,255,.36)" stroke-width="1.4" />
              <path d="M52 74 108 126 28 145 M108 126 183 21 206 104 310 101 M206 104 180 180 330 174 M180 180 80 303 35 230 M180 180 238 317 305 244 M108 126 180 180 206 104 M52 74 206 104 M28 145 180 180 M310 101 330 174 M305 244 180 180 M80 303 238 317" fill="none" stroke="rgba(37,211,255,.31)" stroke-width="1.25" />
              <g fill="#32d5ff" filter="url(#globeGlow)">
                <circle cx="52" cy="74" r="4" /><circle cx="115" cy="32" r="4" /><circle cx="183" cy="21" r="5" /><circle cx="251" cy="47" r="4" /><circle cx="310" cy="101" r="5" /><circle cx="330" cy="174" r="4" /><circle cx="305" cy="244" r="4" /><circle cx="238" cy="317" r="4" /><circle cx="158" cy="336" r="4" /><circle cx="80" cy="303" r="4" /><circle cx="35" cy="230" r="4" /><circle cx="28" cy="145" r="3" /><circle cx="108" cy="126" r="4" /><circle cx="206" cy="104" r="4" /><circle cx="180" cy="180" r="4" />
              </g>
            </svg>
            <div class="network-core-content">${brandMark()}<h2>MundusX <span>Network</span></h2><p>A global decentralized<br />compute network</p></div>
          </div>
          <div class="server-and-panel">
            <div class="server-stack" aria-hidden="true"><span></span><span></span><span></span></div>
            <div class="node-list">
              <div>${siteIcon("cpu")}AI Models</div>
              <div>${siteIcon("database")}Compute Nodes</div>
              <div>${siteIcon("box")}Your Workloads</div>
            </div>
          </div>
        </div>
      </section>
      <section class="values" aria-labelledby="workspace-title">
        <div class="values-inner">
          <h2 id="workspace-title">One workspace. Your AI. Your compute.</h2>
          <p>A more open, more capable and more connected way to work with AI.</p>
          <div class="value-grid">
            <div class="value"><span class="value-icon">${siteIcon("shield")}</span><div><h3>Private by design</h3><p>Your data stays yours. Secure, transparent and under your control.</p></div></div>
            <div class="value"><span class="value-icon">${siteIcon("code")}</span><div><h3>Built for builders</h3><p>From ideas to production. Tools, APIs and compute for real work.</p></div></div>
            <div class="value"><span class="value-icon">${siteIcon("share")}</span><div><h3>Distributed by default</h3><p>A global network of people and compute powering a more open AI future.</p></div></div>
          </div>
          <div class="cta">
            <div>
              <h2>Ready to build with MundusX?</h2>
              <p>Join a growing community of builders, researchers and businesses shaping a more open and decentralized AI future.</p>
            </div>
            <div>
              <a class="btn primary" href="./public/install/">Get started <span aria-hidden="true">-&gt;</span></a>
              <small>Start free. No credit card required.</small>
            </div>
          </div>
        </div>
        <div class="mountains" aria-hidden="true"></div>
      </section>
    </main>
    <footer class="site-footer">
      <div class="footer-inner">
        <a class="brand" href="./" aria-label="MundusX home">${brandMark()}<span>MundusX</span></a>
        <div class="footer-links">
          <a href="./public/docs/">Product</a>
          <a href="./public/release/">Network</a>
          <a href="./docs/">Developers</a>
          <a href="./public/install/">Pricing</a>
          <a href="./docs/device-identity/">About</a>
          <a href="./docs/release/">Blog</a>
        </div>
        <div class="social-links" aria-label="Social links">
          <a href="./public/docs/" aria-label="GitHub">${siteIcon("github")}</a>
          <a href="./public/docs/" aria-label="Community">${siteIcon("discord")}</a>
          <a href="./public/docs/" aria-label="LinkedIn">${siteIcon("linkedin")}</a>
          <a href="./public/docs/" aria-label="Video">${siteIcon("youtube")}</a>
        </div>
        <span class="tagline">A better tomorrow</span>
      </div>
    </footer>
  </body>
</html>`;
}

function brandMark(className = "mark") {
  return `<img class="${className}" src="./assets/mundusx-logo-official.svg" alt="" aria-hidden="true" />`;
}

function siteIcon(name) {
  const common = 'xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"';
  const icons = {
    "arrow-up": '<path d="m12 19 0-14"/><path d="m5 12 7-7 7 7"/>',
    "bar-chart": '<path d="M4 20V10"/><path d="M10 20V4"/><path d="M16 20v-7"/><path d="M22 20V8"/>',
    box: '<path d="m21 8-9-5-9 5 9 5 9-5Z"/><path d="M3 8v8l9 5 9-5V8"/><path d="M12 13v8"/>',
    code: '<path d="m16 18 6-6-6-6"/><path d="m8 6-6 6 6 6"/><path d="m14 4-4 16"/>',
    cpu: '<rect x="5" y="5" width="14" height="14" rx="2"/><rect x="9" y="9" width="6" height="6"/><path d="M9 1v4"/><path d="M15 1v4"/><path d="M9 19v4"/><path d="M15 19v4"/><path d="M1 9h4"/><path d="M1 15h4"/><path d="M19 9h4"/><path d="M19 15h4"/>',
    database: '<ellipse cx="12" cy="5" rx="8" ry="3"/><path d="M4 5v6c0 1.7 3.6 3 8 3s8-1.3 8-3V5"/><path d="M4 11v6c0 1.7 3.6 3 8 3s8-1.3 8-3v-6"/>',
    discord: '<path d="M8 8c2.7-1 5.3-1 8 0"/><path d="M7 17c3.3 1.3 6.7 1.3 10 0"/><path d="M8 8 6 18l3-2"/><path d="m16 8 2 10-3-2"/><path d="M9.5 13h.01"/><path d="M14.5 13h.01"/>',
    folder: '<path d="M3 7.5A2.5 2.5 0 0 1 5.5 5H10l2 2h6.5A2.5 2.5 0 0 1 21 9.5v7A2.5 2.5 0 0 1 18.5 19h-13A2.5 2.5 0 0 1 3 16.5Z"/>',
    github: '<path d="M15 22v-4a4.8 4.8 0 0 0-1-3.5c3.4-.4 7-1.7 7-7.5a5.8 5.8 0 0 0-1.6-4A5.4 5.4 0 0 0 19.3 0S18 0 15 1.5a14.8 14.8 0 0 0-6 0C6 0 4.7 0 4.7 0a5.4 5.4 0 0 0-.1 3A5.8 5.8 0 0 0 3 7c0 5.8 3.6 7.1 7 7.5A4.8 4.8 0 0 0 9 18v4"/><path d="M9 18c-4.5 2-5-2-7-2"/>',
    globe: '<circle cx="12" cy="12" r="10"/><path d="M2 12h20"/><path d="M12 2a15.3 15.3 0 0 1 0 20"/><path d="M12 2a15.3 15.3 0 0 0 0 20"/>',
    laptop: '<path d="M20 16V7a2 2 0 0 0-2-2H6a2 2 0 0 0-2 2v9"/><path d="M2 20h20"/><path d="M6 16h12"/>',
    layers: '<path d="m12 2 9 5-9 5-9-5 9-5Z"/><path d="m3 12 9 5 9-5"/><path d="m3 17 9 5 9-5"/>',
    leaf: '<path d="M11 20A7 7 0 0 1 4 13c0-6 8-10 16-9-1 8-5 16-11 16Z"/><path d="M4 13c4 0 8-1 12-5"/>',
    linkedin: '<path d="M16 8a6 6 0 0 1 6 6v7h-4v-7a2 2 0 0 0-4 0v7h-4v-7a6 6 0 0 1 6-6Z"/><path d="M2 9h4v12H2z"/><circle cx="4" cy="4" r="2"/>',
    "message-square": '<path d="M21 15a4 4 0 0 1-4 4H8l-5 3V7a4 4 0 0 1 4-4h10a4 4 0 0 1 4 4Z"/>',
    mic: '<path d="M12 2a3 3 0 0 0-3 3v7a3 3 0 0 0 6 0V5a3 3 0 0 0-3-3Z"/><path d="M19 10v2a7 7 0 0 1-14 0v-2"/><path d="M12 19v3"/>',
    moon: '<path d="M20.5 14.5A8 8 0 0 1 9.5 3.5 8 8 0 1 0 20.5 14.5Z"/>',
    network: '<circle cx="12" cy="5" r="3"/><circle cx="5" cy="19" r="3"/><circle cx="19" cy="19" r="3"/><path d="M10.6 7.7 6.4 16.3"/><path d="m13.4 7.7 4.2 8.6"/><path d="M8 19h8"/>',
    paperclip: '<path d="m21.4 11.6-8.5 8.5a6 6 0 0 1-8.5-8.5l8.5-8.5a4 4 0 0 1 5.7 5.7l-8.5 8.5a2 2 0 0 1-2.8-2.8l8.5-8.5"/>',
    search: '<circle cx="11" cy="11" r="8"/><path d="m21 21-4.3-4.3"/>',
    share: '<circle cx="18" cy="5" r="3"/><circle cx="6" cy="12" r="3"/><circle cx="18" cy="19" r="3"/><path d="m8.6 10.6 6.8-4.2"/><path d="m8.6 13.4 6.8 4.2"/>',
    shield: '<path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10Z"/><path d="m9 12 2 2 4-5"/>',
    sliders: '<path d="M4 21v-7"/><path d="M4 10V3"/><path d="M12 21v-9"/><path d="M12 8V3"/><path d="M20 21v-5"/><path d="M20 12V3"/><path d="M2 14h4"/><path d="M10 8h4"/><path d="M18 16h4"/>',
    spark: '<path d="m12 3 1.8 5.2L19 10l-5.2 1.8L12 17l-1.8-5.2L5 10l5.2-1.8L12 3Z"/>',
    sun: '<circle cx="12" cy="12" r="4"/><path d="M12 2v2"/><path d="M12 20v2"/><path d="m4.9 4.9 1.4 1.4"/><path d="m17.7 17.7 1.4 1.4"/><path d="M2 12h2"/><path d="M20 12h2"/><path d="m6.3 17.7-1.4 1.4"/><path d="m19.1 4.9-1.4 1.4"/>',
    terminal: '<rect x="3" y="4" width="18" height="16" rx="2"/><path d="m8 9 3 3-3 3"/><path d="M13 15h3"/>',
    users: '<path d="M16 21v-2a4 4 0 0 0-4-4H6a4 4 0 0 0-4 4v2"/><circle cx="9" cy="7" r="4"/><path d="M22 21v-2a4 4 0 0 0-3-3.9"/><path d="M16 3.1a4 4 0 0 1 0 7.8"/>',
    youtube: '<path d="M22.5 8.5a3 3 0 0 0-2.1-2.1C18.5 6 12 6 12 6s-6.5 0-8.4.4a3 3 0 0 0-2.1 2.1A31 31 0 0 0 1 12a31 31 0 0 0 .5 3.5 3 3 0 0 0 2.1 2.1C5.5 18 12 18 12 18s6.5 0 8.4-.4a3 3 0 0 0 2.1-2.1A31 31 0 0 0 23 12a31 31 0 0 0-.5-3.5Z"/><path d="m10 15 5-3-5-3Z"/>',
    zap: '<path d="M13 2 3 14h8l-1 8 10-12h-8l1-8Z"/>',
  };
  return `<svg ${common} aria-hidden="true">${icons[name] ?? icons.spark}</svg>`;
}

function writePublicInstallSurface() {
  const publicDir = path.join(outputDir, "public");
  const installDir = path.join(publicDir, "install");
  fs.mkdirSync(installDir, { recursive: true });

  const manifest = {
    kind: "install-manifest",
    product_name: "MundusX",
    audience: "public",
    install_script_href: "./install.sh",
    install_powershell_href: "./install.ps1",
    install_command: "RELEASE_BASE_URL=http://127.0.0.1:8788/releases/latest/download bash install.sh",
    windows_install_command:
      ".\\install.ps1 -ReleaseBaseUrl http://127.0.0.1:8788/releases/latest/download",
    release_base_url: repoReleaseBaseUrl,
    release_notes_url: repoReleaseNotesUrl,
    docs_home_href: "./docs/",
    install_docs_href: "./docs/install/",
    checksum_hint:
      "The hosted install script uses the signed GitHub release artifacts and fails closed unless checksum and signed manifest artifacts are present.",
  };

  fs.writeFileSync(path.join(publicDir, "install.sh"), installScriptContents);
  fs.writeFileSync(path.join(publicDir, "install.ps1"), installPowershellContents);
  fs.writeFileSync(path.join(publicDir, "install.json"), JSON.stringify(manifest, null, 2) + "\n");
  fs.writeFileSync(path.join(outputDir, "install.json"), JSON.stringify(manifest, null, 2) + "\n");
  fs.writeFileSync(path.join(outputDir, "install", "index.html"), renderLocalInstallPage());
  fs.writeFileSync(path.join(installDir, "index.html"), renderPublicInstallPage());
}

function writePublicReleaseSurface() {
  const publicDir = path.join(outputDir, "public");
  const releaseDir = path.join(publicDir, "release");
  fs.mkdirSync(releaseDir, { recursive: true });

  const manifest = {
    kind: "release-channel",
    product_name: "MundusX",
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
      `<section><h2>Canonical commands</h2><pre><code>RELEASE_BASE_URL=http://127.0.0.1:8788/releases/latest/download bash install.sh
.\\install.ps1 -ReleaseBaseUrl http://127.0.0.1:8788/releases/latest/download</code></pre></section>`,
    );
  }
  if (page.slug === "device-identity") {
    summaryBlocks.push(
      `<section><h2>Current stance</h2><p>MundusX should prefer non-exportable device keys so the CLI, agent, and worker can request signatures without reading raw private-key bytes.</p></section>`,
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
    <title>Install MundusX</title>
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
        <h1>Install MundusX</h1>
        <p class="lead">This Pages-backed install surface mirrors the future public endpoint shape. It hosts the reviewed installer scripts from this repo and points them at the latest signed GitHub release artifacts.</p>
        <div class="actions">
          <a class="primary" href="../install.sh">Download install.sh</a>
          <a href="../install.ps1">Download install.ps1</a>
          <a href="../docs/install/">Install docs</a>
          <a href="../docs/release/">Release notes and distribution</a>
        </div>
      </section>

      <section class="panel">
        <h2>One command</h2>
        <div class="command" id="install-command">Fetching ../install.json</div>
        <p class="manifest" id="manifest-state">The public install surface loads its commands and release source from <code>../install.json</code>.</p>
      </section>

      <section class="panel">
        <h2>What this endpoint guarantees</h2>
        <div class="grid">
          <div class="card">
            <h3>Repo-owned scripts</h3>
            <p>The checked-in <code>install.sh</code> and <code>install.ps1</code> scripts are published under <code>/public</code>.</p>
          </div>
          <div class="card">
            <h3>Signed release source</h3>
            <p>The installer defaults to the latest GitHub release download set and requires the matching checksum plus signed release manifest artifacts.</p>
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
          <li>Use the shell installer on macOS/Linux and the PowerShell installer on Windows.</li>
          <li>The installer places <code>opengpu</code> or <code>opengpu.exe</code> into the platform default bin directory, with <code>mundusx</code> or <code>mundusx.exe</code> retained as a compatibility alias.</li>
          <li>After install, use <code>opengpu install</code> before <code>opengpu start</code> so control plane, cap, and model selection are explicit.</li>
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
        const installPowershellUrl = new URL(manifest.install_powershell_href, manifestUrl).href;
        const command =
          "macOS/Linux: curl -fsSL " +
          installScriptUrl +
          " | bash\nWindows: powershell -ExecutionPolicy Bypass -Command \"iwr " +
          installPowershellUrl +
          " -OutFile install.ps1; .\\install.ps1\"";
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

function renderLocalInstallPage() {
  return renderPublicInstallPage()
    .replaceAll("../install.json", "./install.json")
    .replaceAll("../install.sh", "./public/install.sh")
    .replaceAll("../install.ps1", "./public/install.ps1")
    .replaceAll("../docs/", "./docs/")
    .replaceAll("Public Install Endpoint", "Local Install Preview")
    .replaceAll("This Pages-backed install surface mirrors the future public endpoint shape. It hosts the reviewed installer scripts from this repo and points them at the latest signed GitHub release artifacts.", "This localhost install surface hosts the reviewed installer scripts from this repo and points them at the current release artifacts.")
    .replaceAll("under <code>/public</code>", "under <code>/public</code> for preview download")
    .replaceAll("under <code>/public/docs</code>", "under <code>/docs</code>");
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
            <p>The public install surface and checked-in installer scripts remain the supported way to consume that release channel.</p>
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
