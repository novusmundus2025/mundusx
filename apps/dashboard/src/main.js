import { createServer } from "node:http";

const controlPlaneUrl = process.env.OPENGPU_CONTROL_PLANE_URL ?? "http://127.0.0.1:8787";
const port = Number(process.env.PORT ?? "3001");
const appUrl = `http://127.0.0.1:${port}`;

const formatCount = (value) => new Intl.NumberFormat("en-US").format(Number(value ?? 0));
const formatCredits = (value) => {
  const normalized = Math.abs(Number(value ?? 0)) < 0.000001 ? 0 : Number(value ?? 0);
  return normalized.toFixed(2);
};

const escapeHtml = (input) =>
  String(input ?? "")
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
    .replaceAll("'", "&#39;");

async function fetchJson(path) {
  const controller = new AbortController();
  const timeout = setTimeout(() => controller.abort(), 2500);

  try {
    const response = await fetch(new URL(path, controlPlaneUrl), {
      signal: controller.signal,
      headers: {
        Accept: "application/json",
      },
    });
    if (!response.ok) {
      throw new Error(`HTTP ${response.status}`);
    }
    return await response.json();
  } finally {
    clearTimeout(timeout);
  }
}

function badge(label, tone = "neutral") {
  return `<span class="pill pill-${tone}">${escapeHtml(label)}</span>`;
}

function renderCounts(snapshot = {}) {
  const cards = [
    ["Online", snapshot.online_count, "green"],
    ["Paused", snapshot.paused_count, "orange"],
    ["Policy blocked", snapshot.policy_blocked_count, "red"],
    ["Queued jobs", snapshot.queued_job_count, "blue"],
    ["Assigned jobs", snapshot.assigned_job_count, "amber"],
    ["Completed jobs", snapshot.completed_job_count, "green"],
    ["Failed jobs", snapshot.failed_job_count, "red"],
    ["Job events", snapshot.job_events, "neutral"],
  ];

  return cards
    .map(
      ([label, value, tone]) => `
        <div class="card">
          <div class="card-label">${escapeHtml(label)}</div>
          <div class="card-value">${formatCount(value)}</div>
          <div>${badge(tone === "neutral" ? "live" : tone, tone)}</div>
        </div>`,
    )
    .join("");
}

function renderNodes(nodes = []) {
  if (!nodes.length) {
    return `<div class="empty">No nodes are registered yet.</div>`;
  }

  return `
    <div class="table">
      <div class="thead">
        <div>Node</div>
        <div>Host</div>
        <div>Backend</div>
        <div>State</div>
        <div>Policy</div>
        <div>Power</div>
        <div>Updated</div>
      </div>
      ${nodes
        .map((node) => {
          const battery = node.battery_percent == null ? "unknown" : `${node.battery_percent}%`;
          const power = `${node.power_source ?? "unknown"} • ${node.on_battery ? "battery" : "AC"} • ${battery}`;
          const policyTone = node.policy_allowed ? "green" : "red";
          const stateTone =
            node.state === "ready" ? "green" : node.state === "busy" ? "amber" : node.state === "paused" ? "orange" : "red";
          return `
            <div class="row">
              <div>
                <strong>${escapeHtml(node.node_id)}</strong>
                <div class="meta">fingerprint ${escapeHtml(node.public_key_fingerprint ?? "unknown")}</div>
                <div class="meta">cap ${escapeHtml(node.contribution_percent ?? 0)}%</div>
              </div>
              <div>
                <div>${escapeHtml(node.hostname ?? "unknown")}</div>
                <div class="meta">signed device</div>
              </div>
              <div>${escapeHtml(node.backend ?? "unknown")}</div>
              <div>${badge(node.state ?? "unknown", stateTone)}</div>
              <div>
                ${badge(node.policy_allowed ? "allowed" : "blocked", policyTone)}
                ${node.policy_reason ? `<div class="meta">${escapeHtml(node.policy_reason)}</div>` : ""}
              </div>
              <div>
                <div>${escapeHtml(power)}</div>
                <div class="meta">${escapeHtml(node.available_memory_mb ?? 0)} MB free • ${escapeHtml(node.available_gpu_percent ?? 0)}% GPU free</div>
              </div>
              <div>${escapeHtml(node.updated_at ?? "unknown")}</div>
            </div>`;
        })
        .join("")}
    </div>`;
}

function renderEvents(events = []) {
  if (!events.length) {
    return `<div class="empty">No job events yet.</div>`;
  }

  return `
    <div class="events">
      ${events
        .slice()
        .reverse()
        .slice(0, 24)
        .map(
          (event) => `
            <div class="event">
              <div class="event-top">
                <strong>${escapeHtml(event.event_type)}</strong>
                <span class="meta">${escapeHtml(event.created_at ?? "unknown")}</span>
              </div>
              <div class="meta">node ${escapeHtml(event.node_id ?? "n/a")} • job ${escapeHtml(event.job_id ?? "n/a")}</div>
              <pre>${escapeHtml(JSON.stringify(event.payload ?? {}, null, 2))}</pre>
            </div>`,
        )
        .join("")}
    </div>`;
}

function renderCredits(credits = {}) {
  const ledger = Array.isArray(credits.ledger) ? credits.ledger : [];
  const byNode = credits.by_node ?? {};
  const total = Number(credits.total ?? 0);
  const balanceRows = Object.entries(byNode);

  const balances = balanceRows.length
    ? `
      <div class="balance-grid">
        ${balanceRows
          .map(
            ([nodeId, amount]) => `
              <div class="balance">
                <strong>${escapeHtml(nodeId)}</strong>
                <div class="meta">${formatCount(amount)} credits</div>
              </div>`,
          )
          .join("")}
      </div>`
    : `<div class="empty">No contributor balances yet.</div>`;

  const recentEntries = ledger.length
    ? `
      <div class="events">
        ${ledger
          .slice()
          .reverse()
          .slice(0, 12)
          .map(
            (entry) => `
              <div class="event">
            <div class="event-top">
                <strong>${escapeHtml(entry.entry_type ?? "credit")}</strong>
                <span class="meta">${escapeHtml(entry.created_at ?? "unknown")}</span>
              </div>
              <div class="meta">
                device ${escapeHtml(entry.device_id ?? "n/a")} • job ${escapeHtml(entry.job_id ?? "n/a")} •
                  ${formatCredits(entry.amount ?? 0)} ${escapeHtml(entry.currency ?? "credits")}
              </div>
                ${entry.metadata ? `<pre>${escapeHtml(JSON.stringify(entry.metadata, null, 2))}</pre>` : ""}
              </div>`,
          )
          .join("")}
      </div>`
    : `<div class="empty">No ledger entries recorded yet.</div>`;

  return `
    <div class="section">
      <div class="section-head">
        <h2 class="section-title">Credits</h2>
        <div class="meta">${formatCount(ledger.length)} ledger entries</div>
      </div>
      <div class="section-body">
        <div class="grid credits-grid">
          <div class="card">
            <div class="card-label">Total credits</div>
            <div class="card-value">${formatCredits(total)}</div>
            <div>${badge("ledger live", "green")}</div>
          </div>
          <div class="card">
            <div class="card-label">Contributor balances</div>
            <div class="card-value">${formatCount(balanceRows.length)}</div>
            <div>${badge(balanceRows.length ? "allocated" : "empty", balanceRows.length ? "blue" : "neutral")}</div>
          </div>
        </div>
        <h3 class="subhead">Balances by node</h3>
        ${balances}
        <h3 class="subhead">Recent awards</h3>
        ${recentEntries}
      </div>
    </div>`;
}

function renderInstallPage() {
  return `<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <title>OpenGPU Install</title>
    <style>
      :root {
        color-scheme: dark;
        --bg: #050816;
        --bg-2: #0a1030;
        --panel: rgba(10, 15, 34, 0.72);
        --panel-2: rgba(17, 24, 46, 0.84);
        --line: rgba(163, 184, 255, 0.18);
        --line-strong: rgba(163, 184, 255, 0.32);
        --text: #eff4ff;
        --muted: #9caaca;
        --muted-2: #7180a0;
        --green: #8ef0aa;
        --blue: #a9c8ff;
        --cyan: #89e7ff;
        --amber: #ffd58d;
        --violet: #c7a7ff;
        --shadow: 0 28px 100px rgba(0, 0, 0, 0.45);
      }
      * { box-sizing: border-box; }
      body {
        margin: 0;
        min-height: 100vh;
        background:
          radial-gradient(circle at 15% 15%, rgba(105, 123, 255, 0.34), transparent 0 30%),
          radial-gradient(circle at 85% 18%, rgba(77, 233, 255, 0.18), transparent 0 24%),
          radial-gradient(circle at 60% 78%, rgba(176, 126, 255, 0.14), transparent 0 28%),
          linear-gradient(180deg, var(--bg) 0%, var(--bg-2) 100%);
        color: var(--text);
        font-family:
          "SF Pro Display",
          "SF Pro Text",
          "Segoe UI",
          "Avenir Next",
          "Helvetica Neue",
          sans-serif;
      }
      body::before {
        content: "";
        position: fixed;
        inset: 0;
        pointer-events: none;
        background-image:
          linear-gradient(rgba(255, 255, 255, 0.025) 1px, transparent 1px),
          linear-gradient(90deg, rgba(255, 255, 255, 0.025) 1px, transparent 1px);
        background-size: 72px 72px;
        mask-image: radial-gradient(circle at center, black 45%, transparent 88%);
        opacity: 0.45;
      }
      .wrap {
        max-width: 1240px;
        margin: 0 auto;
        padding: 24px 20px 56px;
      }
      .topbar {
        display: flex;
        align-items: center;
        justify-content: space-between;
        gap: 16px;
        flex-wrap: wrap;
        margin-bottom: 18px;
        color: var(--muted);
        text-transform: uppercase;
        letter-spacing: 0.08em;
        font-size: 12px;
      }
      .brand {
        color: var(--text);
        font-weight: 700;
        letter-spacing: 0.12em;
      }
      .topbar-right {
        display: flex;
        align-items: center;
        gap: 12px;
        flex-wrap: wrap;
      }
      .dot {
        width: 10px;
        height: 10px;
        border-radius: 999px;
        background: var(--green);
        box-shadow: 0 0 24px rgba(142, 240, 170, 0.7);
      }
      .topbar-chip {
        display: inline-flex;
        align-items: center;
        gap: 8px;
        padding: 10px 14px;
        border-radius: 999px;
        background: rgba(255, 255, 255, 0.04);
        border: 1px solid var(--line);
        color: var(--muted);
        font-size: 12px;
        letter-spacing: 0.08em;
        text-transform: uppercase;
      }
      .hero {
        position: relative;
        overflow: hidden;
        border: 1px solid var(--line);
        background:
          linear-gradient(180deg, rgba(17, 24, 43, 0.92), rgba(9, 13, 23, 0.96)),
          linear-gradient(135deg, rgba(92, 126, 255, 0.12), rgba(123, 235, 255, 0.04));
        border-radius: 32px;
        padding: 28px;
        box-shadow: var(--shadow);
        backdrop-filter: blur(16px);
      }
      .hero::after {
        content: "";
        position: absolute;
        inset: 0;
        background:
          radial-gradient(circle at 20% 10%, rgba(120, 143, 255, 0.12), transparent 28%),
          radial-gradient(circle at 86% 16%, rgba(123, 235, 255, 0.11), transparent 22%);
        pointer-events: none;
      }
      h1 {
        margin: 0;
        max-width: 12ch;
        font-size: clamp(56px, 7vw, 92px);
        line-height: 0.92;
        letter-spacing: -0.08em;
        text-transform: uppercase;
      }
      .sub {
        margin-top: 18px;
        color: var(--muted);
        line-height: 1.75;
        max-width: 60ch;
        font-size: 17px;
      }
      .hero-grid {
        display: grid;
        grid-template-columns: minmax(0, 1.15fr) minmax(360px, 0.85fr);
        gap: 24px;
        align-items: start;
        margin-top: 30px;
      }
      .stack {
        display: grid;
        gap: 16px;
        margin-top: 0;
      }
      .panel {
        border: 1px solid var(--line);
        border-radius: 24px;
        background: rgba(8, 12, 21, 0.72);
        padding: 18px;
        box-shadow: inset 0 1px 0 rgba(255, 255, 255, 0.04);
      }
      .label {
        color: var(--muted);
        text-transform: uppercase;
        letter-spacing: 0.08em;
        font-size: 12px;
        margin-bottom: 12px;
      }
      .command {
        display: flex;
        align-items: center;
        justify-content: space-between;
        gap: 14px;
        padding: 20px;
        border-radius: 22px;
        background:
          linear-gradient(180deg, rgba(6, 10, 18, 0.98), rgba(5, 8, 16, 0.98));
        border: 1px solid var(--line-strong);
        font-size: 15px;
        overflow-x: auto;
      }
      code {
        color: #f7fbff;
        white-space: nowrap;
        font-family: "SFMono-Regular", Menlo, Monaco, Consolas, monospace;
      }
      .pill {
        display: inline-flex;
        align-items: center;
        padding: 6px 10px;
        border-radius: 999px;
        font-size: 12px;
        letter-spacing: 0.08em;
        text-transform: uppercase;
        border: 1px solid rgba(169, 200, 255, 0.22);
        color: var(--blue);
        background: rgba(169, 200, 255, 0.08);
      }
      .grid {
        display: grid;
        grid-template-columns: repeat(2, minmax(0, 1fr));
        gap: 16px;
        margin-top: 18px;
      }
      .card {
        border: 1px solid var(--line);
        border-radius: 22px;
        background: linear-gradient(180deg, rgba(16, 23, 39, 0.88), rgba(9, 13, 22, 0.8));
        padding: 18px;
      }
      .card strong {
        display: block;
        margin-bottom: 10px;
        font-size: 13px;
        text-transform: uppercase;
        letter-spacing: 0.08em;
      }
      .card p {
        margin: 0;
        color: var(--muted);
        line-height: 1.7;
      }
      ul {
        margin: 0;
        padding-left: 18px;
        color: var(--muted);
        line-height: 1.7;
      }
      a {
        color: var(--green);
        text-decoration: none;
      }
      a:hover { text-decoration: underline; }
      .footer {
        margin-top: 18px;
        color: var(--muted);
        font-size: 12px;
        line-height: 1.6;
      }
      .steps {
        display: grid;
        gap: 12px;
      }
      .step {
        display: flex;
        gap: 14px;
        padding: 14px 16px;
        border: 1px solid var(--line);
        border-radius: 18px;
        background: rgba(7, 11, 18, 0.68);
      }
      .step-num {
        width: 30px;
        height: 30px;
        border-radius: 999px;
        display: inline-flex;
        align-items: center;
        justify-content: center;
        flex: 0 0 auto;
        font-weight: 700;
        color: var(--green);
        background: rgba(137, 232, 164, 0.1);
        border: 1px solid rgba(137, 232, 164, 0.25);
      }
      .step strong {
        display: block;
        margin-bottom: 4px;
        font-size: 14px;
      }
      .step p {
        margin: 0;
        color: var(--muted);
        line-height: 1.6;
      }
      .command-actions {
        display: inline-flex;
        align-items: center;
        gap: 10px;
        flex: 0 0 auto;
      }
      .copy-btn {
        border: 1px solid rgba(169, 200, 255, 0.22);
        background: rgba(169, 200, 255, 0.1);
        color: var(--text);
        border-radius: 12px;
        padding: 10px 14px;
        font: inherit;
        cursor: pointer;
      }
      .copy-btn:hover {
        border-color: rgba(169, 200, 255, 0.42);
        background: rgba(169, 200, 255, 0.14);
      }
      .micro {
        color: var(--muted-2);
        font-size: 12px;
        margin-top: 8px;
      }
      .split {
        display: grid;
        grid-template-columns: minmax(0, 1fr) minmax(320px, 0.82fr);
        gap: 18px;
        align-items: stretch;
      }
      .hero-copy {
        padding-right: 6px;
      }
      .hero-metrics {
        display: grid;
        gap: 12px;
      }
      .metric {
        border: 1px solid var(--line);
        background: rgba(7, 11, 18, 0.6);
        border-radius: 20px;
        padding: 16px;
      }
      .metric .k {
        display: block;
        color: var(--muted);
        text-transform: uppercase;
        letter-spacing: 0.08em;
        font-size: 11px;
        margin-bottom: 8px;
      }
      .metric .v {
        font-size: 18px;
        color: var(--text);
        line-height: 1.45;
      }
      .hero-badges {
        display: flex;
        flex-wrap: wrap;
        gap: 10px;
        margin-top: 22px;
      }
      .hero-badges .pill {
        padding: 8px 12px;
      }
      .right-card {
        padding: 18px;
        border-radius: 24px;
        border: 1px solid var(--line);
        background:
          radial-gradient(circle at top right, rgba(123, 235, 255, 0.08), transparent 24%),
          linear-gradient(180deg, rgba(16, 23, 39, 0.9), rgba(7, 11, 18, 0.88));
        display: grid;
        gap: 14px;
      }
      .release-badge {
        width: fit-content;
        padding: 7px 12px;
        border-radius: 999px;
        border: 1px solid rgba(123, 235, 255, 0.26);
        color: var(--cyan);
        background: rgba(123, 235, 255, 0.08);
        letter-spacing: 0.08em;
        text-transform: uppercase;
        font-size: 11px;
      }
      .release-stack {
        display: grid;
        gap: 10px;
      }
      .release-line {
        display: flex;
        align-items: center;
        justify-content: space-between;
        gap: 12px;
        padding: 12px 14px;
        border: 1px solid var(--line);
        border-radius: 16px;
        background: rgba(7, 11, 18, 0.6);
      }
      .release-line .left {
        display: grid;
        gap: 3px;
      }
      .release-line .left strong {
        font-size: 14px;
      }
      .release-line .left span {
        color: var(--muted);
        font-size: 12px;
      }
      .eyebrow {
        display: inline-flex;
        align-items: center;
        gap: 8px;
        padding: 6px 10px;
        border-radius: 999px;
        border: 1px solid rgba(143, 161, 210, 0.2);
        color: var(--muted);
        background: rgba(255, 255, 255, 0.02);
        font-size: 12px;
        letter-spacing: 0.08em;
        text-transform: uppercase;
      }
      @media (max-width: 760px) {
        .grid { grid-template-columns: 1fr; }
        .command { align-items: flex-start; flex-direction: column; }
        .command-actions { width: 100%; justify-content: space-between; }
        .hero-grid { grid-template-columns: 1fr; }
        h1 { max-width: none; }
        .split { grid-template-columns: 1fr; }
      }
    </style>
  </head>
  <body>
    <div class="wrap">
      <div class="hero">
        <div class="topbar">
          <div class="brand">OpenGPU Install</div>
          <div class="topbar-right">
            <div class="topbar-chip"><span class="dot"></span> localhost preview</div>
            <div class="topbar-chip">Mac-first release channel</div>
          </div>
        </div>
        <div class="split">
          <div class="hero-copy">
            <h1>Install OpenGPU on your Mac</h1>
            <div class="sub">
              Fast, local-first installation for Apple Silicon. The installer fetches the signed
              release binary, verifies checksums when available, and sets you up for
              <code>opengpu onboarding</code> and <code>opengpu start</code>.
            </div>
            <div class="hero-badges">
              <span class="pill">Apple Silicon first</span>
              <span class="pill">signed binary</span>
              <span class="pill">checksum verified</span>
              <span class="pill">localhost preview</span>
            </div>

            <div class="grid">
              <div class="card">
                <strong>What happens next</strong>
                <p>
                  The installer downloads the matching Apple Silicon release binary and verifies
                  the checksum when available.
                </p>
              </div>
              <div class="card">
                <strong>After install</strong>
                <p>
                  Run <code>opengpu onboarding</code>, then <code>opengpu cap</code>, then
                  <code>opengpu start</code>.
                </p>
              </div>
            </div>
          </div>

          <div class="right-card">
            <div class="release-badge">one command</div>
            <div class="label">Copy the install command</div>
            <div class="command">
              <code>curl -fsSL https://novusx.ai/install | bash</code>
              <div class="command-actions">
                <button class="copy-btn" type="button" onclick="navigator.clipboard.writeText('curl -fsSL https://novusx.ai/install | bash').then(() => { const el = document.getElementById('copy-status'); if (el) el.textContent = 'copied'; }).catch(() => {});">Copy</button>
              </div>
            </div>
            <div class="micro" id="copy-status">local preview only</div>

            <div class="release-stack">
              <div class="release-line">
                <div class="left">
                  <strong>Download</strong>
                  <span>Signed Mac release binary</span>
                </div>
                <div class="pill">step 1</div>
              </div>
              <div class="release-line">
                <div class="left">
                  <strong>Verify</strong>
                  <span>Checksum when published</span>
                </div>
                <div class="pill">step 2</div>
              </div>
              <div class="release-line">
                <div class="left">
                  <strong>Start</strong>
                  <span>Onboarding, cap, then connect</span>
                </div>
                <div class="pill">step 3</div>
              </div>
            </div>
          </div>
        </div>

        <div class="footer">
          Local preview URL: <code>${escapeHtml(appUrl)}/install</code> •
          Docs preview: <code>${escapeHtml(appUrl)}/docs</code>
        </div>
      </div>
    </div>
  </body>
</html>`;
}

function page({ health, status, events, credits, error }) {
  const snapshot = status ?? health?.snapshot ?? {};
  const storageSource = health?.storage_source ?? snapshot.storage_source ?? "unknown";
  const supabase = health?.supabase ?? "unknown";
  const isHealthy = health?.status === "ok";
  const title = "OpenGPU Dashboard";

  return `<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <meta http-equiv="refresh" content="15" />
    <title>${title}</title>
    <style>
      :root {
        color-scheme: dark;
        --bg: #0a0d12;
        --panel: #111620;
        --panel-2: #151c29;
        --line: #253040;
        --text: #e8eefc;
        --muted: #91a0b8;
        --green: #8ef0aa;
        --orange: #ffbf7a;
        --amber: #ffd27f;
        --red: #ff9d9d;
        --blue: #a6c8ff;
      }
      * { box-sizing: border-box; }
      body {
        margin: 0;
        min-height: 100vh;
        background:
          radial-gradient(circle at top left, rgba(80, 120, 255, 0.16), transparent 30%),
          radial-gradient(circle at top right, rgba(80, 255, 180, 0.08), transparent 25%),
          linear-gradient(180deg, #0a0d12 0%, #090b10 100%);
        color: var(--text);
        font-family: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace;
      }
      .wrap {
        max-width: 1380px;
        margin: 0 auto;
        padding: 28px 20px 48px;
      }
      .hero {
        border: 1px solid var(--line);
        background: linear-gradient(180deg, rgba(21, 28, 41, 0.92), rgba(12, 17, 25, 0.92));
        border-radius: 18px;
        padding: 24px;
        box-shadow: 0 20px 70px rgba(0, 0, 0, 0.35);
      }
      .topline {
        display: flex;
        align-items: center;
        justify-content: space-between;
        gap: 20px;
        flex-wrap: wrap;
      }
      h1 {
        margin: 0;
        font-size: 30px;
        letter-spacing: 0.08em;
        text-transform: uppercase;
      }
      .sub {
        margin-top: 10px;
        color: var(--muted);
        line-height: 1.5;
      }
      .statusline {
        display: flex;
        gap: 10px;
        flex-wrap: wrap;
        margin-top: 18px;
      }
      .pill {
        display: inline-flex;
        align-items: center;
        padding: 6px 10px;
        border-radius: 999px;
        font-size: 12px;
        letter-spacing: 0.08em;
        text-transform: uppercase;
        border: 1px solid transparent;
      }
      .pill-green { background: rgba(142, 240, 170, 0.12); color: var(--green); border-color: rgba(142, 240, 170, 0.25); }
      .pill-orange { background: rgba(255, 191, 122, 0.12); color: var(--orange); border-color: rgba(255, 191, 122, 0.25); }
      .pill-amber { background: rgba(255, 210, 127, 0.12); color: var(--amber); border-color: rgba(255, 210, 127, 0.25); }
      .pill-red { background: rgba(255, 157, 157, 0.12); color: var(--red); border-color: rgba(255, 157, 157, 0.25); }
      .pill-blue { background: rgba(166, 200, 255, 0.12); color: var(--blue); border-color: rgba(166, 200, 255, 0.25); }
      .pill-neutral { background: rgba(145, 160, 184, 0.12); color: var(--muted); border-color: rgba(145, 160, 184, 0.25); }
      .grid {
        display: grid;
        grid-template-columns: repeat(4, minmax(0, 1fr));
        gap: 14px;
        margin: 18px 0 24px;
      }
      .card {
        border: 1px solid var(--line);
        background: rgba(17, 22, 32, 0.85);
        border-radius: 16px;
        padding: 16px;
      }
      .card-label {
        color: var(--muted);
        text-transform: uppercase;
        letter-spacing: 0.08em;
        font-size: 12px;
      }
      .card-value {
        margin: 10px 0 12px;
        font-size: 30px;
        font-weight: 700;
      }
      .subhead {
        margin: 0 0 12px;
        color: var(--muted);
        text-transform: uppercase;
        letter-spacing: 0.08em;
        font-size: 12px;
      }
      .section {
        margin-top: 24px;
        border: 1px solid var(--line);
        background: rgba(17, 22, 32, 0.72);
        border-radius: 18px;
        overflow: hidden;
      }
      .section-head {
        padding: 16px 20px;
        border-bottom: 1px solid var(--line);
        display: flex;
        align-items: center;
        justify-content: space-between;
        gap: 16px;
        flex-wrap: wrap;
      }
      .section-title {
        margin: 0;
        text-transform: uppercase;
        letter-spacing: 0.08em;
        font-size: 14px;
      }
      .section-body {
        padding: 20px;
      }
      .table .thead,
      .table .row {
        display: grid;
        grid-template-columns: 1.3fr 1fr 0.7fr 0.7fr 1fr 1.1fr 0.7fr;
        gap: 14px;
        align-items: start;
      }
      .table .thead {
        color: var(--muted);
        text-transform: uppercase;
        letter-spacing: 0.08em;
        font-size: 12px;
        padding-bottom: 12px;
        margin-bottom: 12px;
        border-bottom: 1px solid var(--line);
      }
      .table .row {
        padding: 14px 0;
        border-bottom: 1px solid rgba(37, 48, 64, 0.65);
      }
      .table .row:last-child { border-bottom: 0; }
      .meta {
        color: var(--muted);
        font-size: 12px;
        line-height: 1.4;
      }
      .empty {
        color: var(--muted);
        padding: 28px 0;
      }
      .events {
        display: grid;
        gap: 12px;
      }
      .balance-grid {
        display: grid;
        grid-template-columns: repeat(auto-fit, minmax(220px, 1fr));
        gap: 12px;
        margin-bottom: 18px;
      }
      .balance {
        border: 1px solid var(--line);
        border-radius: 14px;
        background: rgba(10, 13, 18, 0.6);
        padding: 14px 16px;
      }
      .balance strong {
        display: block;
        margin-bottom: 6px;
        font-size: 14px;
        color: #f4f8ff;
        overflow-wrap: anywhere;
      }
      .event {
        border: 1px solid var(--line);
        border-radius: 14px;
        background: rgba(10, 13, 18, 0.6);
        padding: 14px 16px;
      }
      .event-top {
        display: flex;
        align-items: center;
        justify-content: space-between;
        gap: 12px;
        flex-wrap: wrap;
      }
      pre {
        overflow: auto;
        margin: 12px 0 0;
        color: #dbe4f5;
        font-size: 12px;
        line-height: 1.5;
        white-space: pre-wrap;
        word-break: break-word;
      }
      .error {
        margin-top: 18px;
        border: 1px solid rgba(255, 157, 157, 0.28);
        background: rgba(255, 157, 157, 0.08);
        color: var(--red);
        padding: 14px 16px;
        border-radius: 14px;
      }
      .links {
        display: flex;
        gap: 10px;
        flex-wrap: wrap;
      }
      a {
        color: var(--blue);
        text-decoration: none;
      }
      a:hover { text-decoration: underline; }
      @media (max-width: 1200px) {
        .grid { grid-template-columns: repeat(2, minmax(0, 1fr)); }
        .table .thead,
        .table .row { grid-template-columns: 1.1fr 0.9fr 0.7fr 0.7fr 1fr 1fr 0.7fr; }
      }
      @media (max-width: 820px) {
        .grid { grid-template-columns: 1fr; }
        .table .thead { display: none; }
        .table .row {
          grid-template-columns: 1fr;
          gap: 10px;
          padding: 16px 0;
        }
      }
    </style>
  </head>
  <body>
    <div class="wrap">
      <div class="hero">
        <div class="topline">
          <div>
            <h1>OpenGPU Dashboard</h1>
            <div class="sub">Live operator view for nodes, jobs, storage source, and audit trail.</div>
            <div class="statusline">
              ${badge(isHealthy ? "healthy" : "degraded", isHealthy ? "green" : "red")}
              ${badge(`storage: ${storageSource}`, storageSource === "supabase" ? "green" : "amber")}
              ${badge(`supabase: ${supabase}`, supabase.startsWith("enabled") ? "green" : "red")}
            </div>
          </div>
          <div class="links">
            <a href="${escapeHtml(appUrl)}/docs" target="_blank" rel="noreferrer">docs</a>
            <a href="${escapeHtml(appUrl)}/install" target="_blank" rel="noreferrer">install</a>
            <a href="${escapeHtml(controlPlaneUrl)}" target="_blank" rel="noreferrer">control plane</a>
            <a href="${escapeHtml(controlPlaneUrl)}/v1/status" target="_blank" rel="noreferrer">status json</a>
            <a href="${escapeHtml(controlPlaneUrl)}/v1/job-events" target="_blank" rel="noreferrer">job events</a>
            <a href="${escapeHtml(controlPlaneUrl)}/health" target="_blank" rel="noreferrer">health</a>
          </div>
        </div>

        <div class="grid">
          ${renderCounts(snapshot)}
        </div>

        ${error ? `<div class="error">${escapeHtml(error)}</div>` : ""}
      </div>

      <div class="section">
        <div class="section-head">
          <h2 class="section-title">Nodes</h2>
          <div class="meta">${formatCount(snapshot.nodes?.length ?? 0)} registered</div>
        </div>
        <div class="section-body">${renderNodes(snapshot.nodes ?? [])}</div>
      </div>

      <div class="section">
        <div class="section-head">
          <h2 class="section-title">Job Events</h2>
          <div class="meta">${formatCount(events.length)} events captured</div>
        </div>
        <div class="section-body">${renderEvents(events)}</div>
      </div>

      ${renderCredits(credits ?? {})}
    </div>
  </body>
</html>`;
}

function docsShell({ title, subtitle, active, body }) {
  const sections = [
    ["Overview", "/docs", "overview"],
    ["Install", "/docs/install", "install"],
    ["Device identity", "/docs/identity", "identity"],
    ["Onboarding", "/docs/onboarding", "onboarding"],
    ["Credits", "/docs/credits", "credits"],
    ["Releases", "/docs/releases", "releases"],
  ];

  return `<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <title>${escapeHtml(title)}</title>
    <style>
      :root {
        color-scheme: dark;
        --bg: #071017;
        --panel: #101826;
        --panel-2: #151f30;
        --line: #243145;
        --text: #ecf3ff;
        --muted: #97a7c0;
        --green: #8ef0aa;
        --blue: #a6c8ff;
        --amber: #ffd27f;
      }
      * { box-sizing: border-box; }
      body {
        margin: 0;
        min-height: 100vh;
        background:
          radial-gradient(circle at top left, rgba(86, 125, 255, 0.16), transparent 28%),
          radial-gradient(circle at top right, rgba(90, 255, 180, 0.08), transparent 24%),
          linear-gradient(180deg, #071017 0%, #090d14 100%);
        color: var(--text);
        font-family: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace;
      }
      .wrap {
        max-width: 1300px;
        margin: 0 auto;
        padding: 28px 20px 48px;
      }
      .topbar {
        display: flex;
        align-items: center;
        justify-content: space-between;
        gap: 16px;
        flex-wrap: wrap;
        margin-bottom: 18px;
      }
      .brand {
        font-weight: 800;
        letter-spacing: 0.08em;
        text-transform: uppercase;
      }
      .badge {
        display: inline-flex;
        align-items: center;
        padding: 6px 10px;
        border-radius: 999px;
        font-size: 12px;
        letter-spacing: 0.08em;
        text-transform: uppercase;
        border: 1px solid rgba(166, 200, 255, 0.22);
        color: var(--blue);
        background: rgba(166, 200, 255, 0.1);
      }
      .layout {
        display: grid;
        grid-template-columns: 280px minmax(0, 1fr);
        gap: 18px;
      }
      .sidebar,
      .content {
        border: 1px solid var(--line);
        border-radius: 18px;
        background: rgba(17, 24, 38, 0.78);
      }
      .sidebar {
        padding: 18px;
        position: sticky;
        top: 20px;
        height: fit-content;
      }
      .content {
        padding: 24px;
        box-shadow: 0 20px 70px rgba(0, 0, 0, 0.28);
      }
      .nav {
        display: grid;
        gap: 8px;
        margin-top: 16px;
      }
      .nav a {
        display: block;
        padding: 12px 14px;
        border-radius: 12px;
        color: var(--text);
        text-decoration: none;
        border: 1px solid transparent;
        background: rgba(7, 12, 18, 0.45);
      }
      .nav a:hover {
        border-color: rgba(166, 200, 255, 0.22);
      }
      .nav a.active {
        border-color: rgba(142, 240, 170, 0.3);
        background: rgba(142, 240, 170, 0.09);
      }
      h1 {
        margin: 0;
        font-size: 34px;
        letter-spacing: 0.08em;
        text-transform: uppercase;
      }
      .subtitle {
        margin-top: 10px;
        color: var(--muted);
        line-height: 1.6;
        max-width: 74ch;
      }
      .cards {
        display: grid;
        grid-template-columns: repeat(2, minmax(0, 1fr));
        gap: 16px;
        margin-top: 20px;
      }
      .card {
        border: 1px solid var(--line);
        border-radius: 16px;
        background: rgba(9, 13, 20, 0.74);
        padding: 18px;
      }
      .card h2,
      .card h3 {
        margin: 0 0 10px;
        font-size: 14px;
        text-transform: uppercase;
        letter-spacing: 0.08em;
      }
      .card p,
      .card li,
      .card ol {
        color: var(--muted);
        line-height: 1.7;
      }
      .card ul,
      .card ol {
        margin: 0;
        padding-left: 18px;
      }
      .mono {
        color: #f4f8ff;
        white-space: nowrap;
      }
      a {
        color: var(--green);
        text-decoration: none;
      }
      a:hover { text-decoration: underline; }
      code {
        color: #f7fbff;
        white-space: nowrap;
      }
      .footer {
        margin-top: 18px;
        color: var(--muted);
        font-size: 12px;
        line-height: 1.6;
      }
      @media (max-width: 900px) {
        .layout { grid-template-columns: 1fr; }
        .sidebar { position: static; }
        .cards { grid-template-columns: 1fr; }
      }
    </style>
  </head>
  <body>
    <div class="wrap">
      <div class="topbar">
        <div class="brand">OpenGPU Docs</div>
        <div class="badge">localhost preview • public layout</div>
      </div>
      <div class="layout">
        <aside class="sidebar">
          <div class="mono">Mac-first docs</div>
          <div class="nav">
            ${sections
              .map(
                ([label, href, key]) => `
                  <a class="${active === key ? "active" : ""}" href="${escapeHtml(href)}">${escapeHtml(label)}</a>`,
              )
              .join("")}
          </div>
          <div class="footer">
            This local site mirrors the public docs shape before the public domain is wired.
          </div>
        </aside>
        <main class="content">
          <h1>${escapeHtml(title)}</h1>
          <div class="subtitle">${subtitle}</div>
          ${body}
          <div class="footer">
            Local preview URL: <code>${escapeHtml(appUrl)}</code>
          </div>
        </main>
      </div>
    </div>
  </body>
</html>`;
}

function renderDocsHome() {
  return docsShell({
    title: "OpenGPU Docs",
    subtitle:
      "A Mac-first public docs surface for install, identity, onboarding, credits, and release flow. This preview is local, but the copy is written as the public source of truth.",
    active: "overview",
    body: `
      <div class="cards">
        <div class="card">
          <h2>Install</h2>
          <p>
            One-line install command, checksum verification, and what the contributor sees
            after download.
          </p>
          <p><a href="/docs/install">Open install page</a></p>
        </div>
        <div class="card">
          <h2>Device identity</h2>
          <p>
            Explain how the Mac identity survives reinstall, why the private key is not
            exportable, and what metadata is signed.
          </p>
          <p><a href="/docs/identity">Open identity page</a></p>
        </div>
        <div class="card">
          <h2>Onboarding</h2>
          <p>
            The first-run checklist for a contributor machine: identity, cap, model, and
            start flow.
          </p>
          <p><a href="/docs/onboarding">Open onboarding page</a></p>
        </div>
        <div class="card">
          <h2>Credits</h2>
          <p>
            Append-only ledger rules for contributor balances and how job awards are tracked.
          </p>
          <p><a href="/docs/credits">Open credits page</a></p>
        </div>
        <div class="card">
          <h2>Releases</h2>
          <p>
            Mac-first public install flow, signed artifacts, and how the release page maps to
            the installer.
          </p>
          <p><a href="/docs/releases">Open releases page</a></p>
        </div>
        <div class="card">
          <h2>Dashboard</h2>
          <p>
            Operator view, live status, and the install preview are still available in the
            dashboard app.
          </p>
          <p><a href="/">Open dashboard</a></p>
        </div>
      </div>
    `,
  });
}

function renderDocsInstall() {
  return docsShell({
    title: "Install OpenGPU",
    subtitle:
      "The install page is the first touch for contributors. It stays Mac-first, keeps the command identical everywhere, and points to onboarding and cap selection immediately after install.",
    active: "install",
    body: `
      <div class="cards">
        <div class="card">
          <h2>Canonical command</h2>
          <p><code>curl -fsSL https://novusx.ai/install | bash</code></p>
          <p>That command should match the installer script, the docs, and the public page.</p>
        </div>
        <div class="card">
          <h2>Expected flow</h2>
          <ol>
            <li>Download the Mac-first release binary.</li>
            <li>Verify checksum when available.</li>
            <li>Run <code>opengpu onboarding</code>.</li>
            <li>Choose a contribution cap with <code>opengpu cap</code>.</li>
            <li>Start with <code>opengpu start</code>.</li>
          </ol>
        </div>
      </div>
    `,
  });
}

function renderDocsIdentity() {
  return docsShell({
    title: "Device Identity",
    subtitle:
      "The Mac identity is a sign-only encrypted-at-rest fallback today. The app never reads raw private-key bytes, and reinstall should reuse identity as long as the OpenGPU data directory remains intact.",
    active: "identity",
    body: `
      <div class="cards">
        <div class="card">
          <h2>What survives reinstall</h2>
          <ul>
            <li>identity record</li>
            <li>public key</li>
            <li>fingerprint</li>
            <li>hostname metadata</li>
          </ul>
        </div>
        <div class="card">
          <h2>What is not exposed</h2>
          <ul>
            <li>raw private key bytes</li>
            <li>exportable app-visible secret</li>
            <li>hostname as identity proof</li>
          </ul>
        </div>
      </div>
    `,
  });
}

function renderDocsOnboarding() {
  return docsShell({
    title: "Onboarding",
    subtitle:
      "The first-run checklist keeps the Mac-first path understandable: review identity, choose a cap, confirm the model, and only then go live.",
    active: "onboarding",
    body: `
      <div class="cards">
        <div class="card">
          <h2>Checklist</h2>
          <ol>
            <li>Review device identity.</li>
            <li>Choose the contribution cap.</li>
            <li>Confirm the active model.</li>
            <li>Run <code>opengpu start</code>.</li>
          </ol>
        </div>
        <div class="card">
          <h2>Policy note</h2>
          <p>The node should remain paused until the cap is set and the policy allows work.</p>
        </div>
      </div>
    `,
  });
}

function renderDocsCredits() {
  return docsShell({
    title: "Credits",
    subtitle:
      "Credits are tracked as an append-only ledger on the control plane. They belong to the contributor account, not the hostname or the device key itself.",
    active: "credits",
    body: `
      <div class="cards">
        <div class="card">
          <h2>Ledger rules</h2>
          <ul>
            <li>job completion writes a ledger entry</li>
            <li>balances roll up by contributor account</li>
            <li>device and hostname stay attached for audit</li>
          </ul>
        </div>
        <div class="card">
          <h2>What users should expect</h2>
          <p>OpenGPU should show earned credits clearly and make the contributor balance easy to inspect in the dashboard.</p>
        </div>
      </div>
    `,
  });
}

function renderDocsReleases() {
  return docsShell({
    title: "Releases",
    subtitle:
      "The public release surface stays Mac-first for now. The public install page, installer script, and signed binary artifacts should always agree on the same release source.",
    active: "releases",
    body: `
      <div class="cards">
        <div class="card">
          <h2>Source of truth</h2>
          <p>The install command, checksum, and release asset must point at the same Mac-first build.</p>
        </div>
        <div class="card">
          <h2>Review rule</h2>
          <p>Any release-page copy change should be checked against the installer script and release docs.</p>
        </div>
      </div>
    `,
  });
}

async function collectData() {
  const [health, status, events, credits] = await Promise.all([
    fetchJson("/health"),
    fetchJson("/v1/status"),
    fetchJson("/v1/job-events"),
    fetchJson("/v1/credits"),
  ]);

  return { health, status, events, credits, error: null };
}

createServer(async (req, res) => {
  const requestUrl = new URL(req.url ?? "/", appUrl);
  if (requestUrl.pathname === "/install") {
    res.writeHead(200, { "Content-Type": "text/html; charset=utf-8" });
    res.end(renderInstallPage());
    return;
  }

  if (requestUrl.pathname === "/docs" || requestUrl.pathname === "/docs/") {
    res.writeHead(200, { "Content-Type": "text/html; charset=utf-8" });
    res.end(renderDocsHome());
    return;
  }

  if (requestUrl.pathname === "/docs/install") {
    res.writeHead(200, { "Content-Type": "text/html; charset=utf-8" });
    res.end(renderDocsInstall());
    return;
  }

  if (requestUrl.pathname === "/docs/identity") {
    res.writeHead(200, { "Content-Type": "text/html; charset=utf-8" });
    res.end(renderDocsIdentity());
    return;
  }

  if (requestUrl.pathname === "/docs/onboarding") {
    res.writeHead(200, { "Content-Type": "text/html; charset=utf-8" });
    res.end(renderDocsOnboarding());
    return;
  }

  if (requestUrl.pathname === "/docs/credits") {
    res.writeHead(200, { "Content-Type": "text/html; charset=utf-8" });
    res.end(renderDocsCredits());
    return;
  }

  if (requestUrl.pathname === "/docs/releases") {
    res.writeHead(200, { "Content-Type": "text/html; charset=utf-8" });
    res.end(renderDocsReleases());
    return;
  }

  let data;
  try {
    data = await collectData();
  } catch (error) {
    data = {
      health: null,
      status: null,
      events: [],
      credits: null,
      error: error instanceof Error ? error.message : String(error),
    };
  }

  res.writeHead(200, { "Content-Type": "text/html; charset=utf-8" });
  res.end(page(data));
}).listen(port, "127.0.0.1", () => {
  process.stdout.write(
    `OpenGPU dashboard listening on http://127.0.0.1:${port} (proxying ${controlPlaneUrl})\n`,
  );
});
