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
        --bg: #070b11;
        --panel: #111a28;
        --panel-2: #151f30;
        --line: #283649;
        --text: #e8eefc;
        --muted: #95a5bc;
        --green: #89e8a4;
        --blue: #a9c8ff;
        --amber: #ffd58d;
      }
      * { box-sizing: border-box; }
      body {
        margin: 0;
        min-height: 100vh;
        background:
          radial-gradient(circle at top left, rgba(100, 140, 255, 0.18), transparent 30%),
          radial-gradient(circle at top right, rgba(120, 255, 195, 0.08), transparent 28%),
          linear-gradient(180deg, #070b11 0%, #090d14 100%);
        color: var(--text);
        font-family: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace;
      }
      .wrap {
        max-width: 1120px;
        margin: 0 auto;
        padding: 32px 20px 56px;
      }
      .topbar {
        display: flex;
        align-items: center;
        justify-content: space-between;
        gap: 16px;
        flex-wrap: wrap;
        margin-bottom: 16px;
        color: var(--muted);
        text-transform: uppercase;
        letter-spacing: 0.08em;
        font-size: 12px;
      }
      .brand {
        color: var(--text);
        font-weight: 700;
      }
      .hero {
        border: 1px solid var(--line);
        background: linear-gradient(180deg, rgba(21, 31, 48, 0.95), rgba(12, 17, 27, 0.95));
        border-radius: 22px;
        padding: 28px;
        box-shadow: 0 24px 80px rgba(0, 0, 0, 0.36);
      }
      h1 {
        margin: 0;
        font-size: 34px;
        letter-spacing: 0.1em;
        text-transform: uppercase;
      }
      .sub {
        margin-top: 12px;
        color: var(--muted);
        line-height: 1.6;
        max-width: 72ch;
      }
      .hero-grid {
        display: grid;
        grid-template-columns: minmax(0, 1.2fr) minmax(300px, 0.8fr);
        gap: 18px;
        align-items: start;
        margin-top: 24px;
      }
      .stack {
        display: grid;
        gap: 16px;
        margin-top: 0;
      }
      .panel {
        border: 1px solid var(--line);
        border-radius: 18px;
        background: rgba(10, 15, 24, 0.72);
        padding: 18px 20px;
      }
      .label {
        color: var(--muted);
        text-transform: uppercase;
        letter-spacing: 0.08em;
        font-size: 12px;
        margin-bottom: 10px;
      }
      .command {
        display: flex;
        align-items: center;
        justify-content: space-between;
        gap: 14px;
        padding: 16px 18px;
        border-radius: 14px;
        background: rgba(6, 10, 16, 0.9);
        border: 1px solid rgba(169, 200, 255, 0.22);
        font-size: 16px;
        overflow-x: auto;
      }
      code {
        color: #f7fbff;
        white-space: nowrap;
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
        background: rgba(169, 200, 255, 0.1);
      }
      .grid {
        display: grid;
        grid-template-columns: repeat(2, minmax(0, 1fr));
        gap: 16px;
        margin-top: 22px;
      }
      .card {
        border: 1px solid var(--line);
        border-radius: 16px;
        background: rgba(17, 26, 40, 0.75);
        padding: 18px;
      }
      .card strong {
        display: block;
        margin-bottom: 8px;
        font-size: 14px;
        text-transform: uppercase;
        letter-spacing: 0.08em;
      }
      .card p {
        margin: 0;
        color: var(--muted);
        line-height: 1.6;
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
        border-radius: 14px;
        background: rgba(6, 10, 16, 0.64);
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
      @media (max-width: 760px) {
        .grid { grid-template-columns: 1fr; }
        .command { align-items: flex-start; flex-direction: column; }
        .hero-grid { grid-template-columns: 1fr; }
      }
    </style>
  </head>
  <body>
    <div class="wrap">
      <div class="hero">
        <div class="topbar">
          <div class="brand">OpenGPU Install</div>
          <div>Mac-first release channel</div>
        </div>
        <div class="pill">localhost preview • mirrors the future public page</div>
        <h1>Install OpenGPU on your Mac</h1>
        <div class="sub">
          Get the Mac-first OpenGPU CLI with one command. The installer downloads the signed
          release binary, verifies checksums when available, and sets you up for
          <code>opengpu onboarding</code> and <code>opengpu start</code>.
        </div>

        <div class="hero-grid">
          <div class="stack">
            <div class="panel">
              <div class="label">Copy the install command</div>
              <div class="command">
                <code>curl -fsSL https://novusx.ai/install | bash</code>
                <span class="pill">one command</span>
              </div>
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
                <strong>What this page is</strong>
                <p>
                  A public-facing landing page for first-time users. It is not the installer
                  itself, and it does not replace the release artifacts.
                </p>
              </div>
              <div class="card">
                <strong>After install</strong>
                <p>
                  Run <code>opengpu onboarding</code> to review the contributor checklist, then
                  <code>opengpu start</code> to bring the machine online.
                </p>
              </div>
              <div class="card">
                <strong>Release discipline</strong>
                <p>
                  The command, checksum, and release asset should always point to the same signed
                  Mac-first release.
                </p>
              </div>
            </div>
          </div>

          <div class="stack">
            <div class="panel">
              <div class="label">How it works</div>
              <div class="steps">
                <div class="step">
                  <div class="step-num">1</div>
                  <div>
                    <strong>Download</strong>
                    <p>The installer fetches the signed release binary for Apple Silicon.</p>
                  </div>
                </div>
                <div class="step">
                  <div class="step-num">2</div>
                  <div>
                    <strong>Verify</strong>
                    <p>When a checksum is published, the installer verifies it before install.</p>
                  </div>
                </div>
                <div class="step">
                  <div class="step-num">3</div>
                  <div>
                    <strong>Start</strong>
                    <p>Use onboarding to review the contributor setup, then bring the node online.</p>
                  </div>
                </div>
              </div>
            </div>

            <div class="panel">
              <div class="label">Mac-first checklist</div>
              <ul>
                <li>The installer command should stay identical everywhere.</li>
                <li>The public page should always match the release assets.</li>
                <li>Checksum verification should stay visible to users.</li>
              </ul>
            </div>
          </div>
        </div>

        <div class="footer">
          Local preview URL: <code>${escapeHtml(appUrl)}/install</code> •
          Docs live in the repository alongside the installer and release workflow.
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
